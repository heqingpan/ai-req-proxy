use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::sync::atomic::Ordering::Relaxed;
use std::{fs, io, path::Path, sync::atomic::AtomicI64};

use actix_web::{App, Error, HttpRequest, HttpResponse, HttpServer, error, middleware, web};
use clap::Parser;
use futures_util::StreamExt as _;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use url::Url;

lazy_static::lazy_static! {
    pub static ref last_req_id: AtomicI64 = AtomicI64::new(0);
}

#[derive(Debug, Serialize, Deserialize)]
struct ToolCall {
    id: Option<String>,
    r#type: Option<String>,
    function: Option<Function>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Function {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    content: serde_json::Value,
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIRequest {
    messages: Vec<Message>,
    tools: Option<serde_json::Value>,
}

pub struct HeaderWrap<'a> {
    pub header_map: &'a actix_http::header::HeaderMap,
}

impl<'a> HeaderWrap<'a> {
    pub fn new(header_map: &'a actix_http::header::HeaderMap) -> Self {
        Self { header_map }
    }
}

impl<'a> Display for HeaderWrap<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (header_name, header_value) in self.header_map.iter() {
            writeln!(
                f,
                "\t{}: {}",
                header_name.as_str(),
                String::from_utf8_lossy(header_value.as_bytes())
            )
            .ok();
        }
        Ok(())
    }
}

pub struct HeaderWrap2<'a> {
    pub header_map: &'a reqwest::header::HeaderMap,
}

impl<'a> HeaderWrap2<'a> {
    pub fn new(header_map: &'a reqwest::header::HeaderMap) -> Self {
        Self { header_map }
    }
}

impl<'a> Display for HeaderWrap2<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (header_name, header_value) in self.header_map.iter() {
            writeln!(
                f,
                "\t{}: {}",
                header_name.as_str(),
                String::from_utf8_lossy(header_value.as_bytes())
            )
            .ok();
        }
        Ok(())
    }
}

fn process_content(content: &serde_json::Value, role: &str) -> String {
    match content {
        serde_json::Value::Array(arr) => {
            let mut result = String::new();
            for (i, item) in arr.iter().enumerate() {
                result.push_str(&format!("\n--- item {} text ---\n\n", i + 1));
                match item {
                    serde_json::Value::Object(obj) => {
                        if let Some(text) = obj.get("text") {
                            result.push_str(&text.to_string());
                        } else {
                            result
                                .push_str(&serde_json::to_string_pretty(item).unwrap_or_default());
                        }
                    }
                    serde_json::Value::String(obj) => {
                        result.push_str(&obj);
                    }
                    _ => {
                        result.push_str(
                            &serde_json::to_string_pretty(item)
                                .unwrap_or_else(|_| item.to_string()),
                        );
                    }
                }
            }
            result
        }
        serde_json::Value::Object(obj) => {
            if let Some(text) = obj.get("text") {
                text.to_string()
            } else {
                serde_json::to_string_pretty(content).unwrap_or_default()
            }
        }
        serde_json::Value::String(s) => {
            if role == "tool" {
                match serde_json::from_str::<serde_json::Value>(s) {
                    Ok(parsed) => {
                        serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| s.clone())
                    }
                    Err(_) => s.clone(),
                }
            } else {
                s.to_string()
            }
        }
        _ => content.to_string(),
    }
}

fn generate_structured_content(request: &OpenAIRequest) -> String {
    let mut content = String::new();

    if let Some(tools) = &request.tools {
        content.push_str("=== TOOLS ===\n");
        content.push_str(&serde_json::to_string_pretty(tools).unwrap_or_default());
        content.push_str("\n\n");
    }

    for (i, message) in request.messages.iter().enumerate() {
        content.push_str(&format!("=== Message {} ({}) ===\n", i + 1, message.role));
        content.push_str(&process_content(&message.content, &message.role));

        if message.role == "assistant" {
            if let Some(tool_calls) = &message.tool_calls {
                if !tool_calls.is_empty() {
                    content.push_str("\n\n=======\n\n");
                    content.push_str(&serde_json::to_string_pretty(tool_calls).unwrap_or_default());
                }
            }
        }
        content.push_str("\n\n");
    }

    content
}

/// 保存请求内容到文件
async fn save_request_to_file(
    request_id: i64,
    date: &str,
    time: &str,
    request_body: &[u8],
) -> Result<(), std::io::Error> {
    let dir_path = Path::new("data").join("req").join(date);
    fs::create_dir_all(&dir_path)?;

    let file_name = format!("{}_{}_{:06}.json", date, time, request_id);
    let file_path = dir_path.join(&file_name);
    fs::write(&file_path, request_body)?;
    log::info!("[req_{}] 请求内容已保存到文件: {:?}", request_id, file_path);

    if let Ok(request) = serde_json::from_slice::<OpenAIRequest>(request_body) {
        let structured_content = generate_structured_content(&request);
        let txt_file_name = format!("{}_{}_{:06}.struct_req.txt", date, time, request_id);
        let txt_file_path = dir_path.join(&txt_file_name);
        fs::write(&txt_file_path, structured_content)?;
        log::info!(
            "[req_{}] 结构化内容已保存到文件: {:?}",
            request_id,
            txt_file_path
        );
    }

    Ok(())
}

/// 保存响应内容到文件
async fn save_response_to_file(
    request_id: i64,
    date: &str,
    time: &str,
    response_body: &[u8],
) -> Result<(), std::io::Error> {
    // 创建目录结构 data/resp/{date}/
    let dir_path = Path::new("data").join("req").join(date);
    fs::create_dir_all(&dir_path)?;

    // 生成文件名 {date}_{time}_{request_id}.resp.json
    let file_name = format!("{}_{}_{:06}.resp.json", date, time, request_id);
    let file_path = dir_path.join(file_name);

    // 保存响应内容到文件
    fs::write(&file_path, response_body)?;

    log::info!("[req_{}] 响应内容已保存到文件: {:?}", request_id, file_path);

    Ok(())
}

/// Same as `forward` but uses `reqwest` as the client used to forward the request.
async fn forward_reqwest(
    req: HttpRequest,
    mut payload: web::Payload,
    method: actix_web::http::Method,
    //peer_addr: Option<PeerAddr>,
    url: web::Data<Url>,
    client: web::Data<reqwest::Client>,
    save_all_requests: web::Data<bool>,
) -> Result<HttpResponse, Error> {
    let request_id = last_req_id.fetch_add(1, Relaxed);
    let now: DateTime<Local> = Local::now();
    let date = now.format("%Y%m%d").to_string();
    let time = now.format("%H%M%S").to_string();
    let path = req.uri().path();

    let mut new_url = (**url).clone();
    new_url.set_path(path);
    new_url.set_query(req.uri().query());

    log::info!(
        "[req_{}] {} {}\n\theaders:\n{}\n",
        request_id,
        req.method().as_str(),
        req.uri(),
        HeaderWrap::new(req.headers())
    );

    let (tx, rx) = mpsc::unbounded_channel();
    let request_id_copy = request_id;
    let save_requests = **save_all_requests;
    let date_copy = date.clone();
    let time_copy = time.clone();
    let mut request_body = Vec::<u8>::new();

    actix_web::rt::spawn(async move {
        while let Some(chunk) = payload.next().await {
            if let Ok(v) = chunk.as_ref() {
                let chunk_bytes = v.as_ref();
                log::info!(
                    "[req_{}] chunk:\n\t{}",
                    request_id_copy,
                    String::from_utf8_lossy(chunk_bytes)
                );

                // 如果启用请求保存，收集请求体内容
                if save_requests {
                    request_body.extend_from_slice(chunk_bytes);
                }
            }
            tx.send(chunk).unwrap();
        }

        // 如果启用请求保存，保存请求内容到文件
        if save_requests && !request_body.is_empty() {
            if let Err(e) =
                save_request_to_file(request_id_copy, &date_copy, &time_copy, &request_body).await
            {
                log::error!("[req_{}] 保存请求内容失败: {}", request_id_copy, e);
            }
        }
    });

    let mut forwarded_req = client
        .request(
            reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap(),
            new_url.clone(),
        )
        .body(reqwest::Body::wrap_stream(UnboundedReceiverStream::new(rx)));

    for (header_name, header_value) in req.headers() {
        if header_name.as_str().eq_ignore_ascii_case("host")
            || header_name.as_str().eq_ignore_ascii_case("accept-encoding")
        {
            continue;
        }
        forwarded_req = forwarded_req.header(header_name.as_str(), header_value.as_ref());
    }

    let res = forwarded_req
        .send()
        .await
        .map_err(error::ErrorInternalServerError)?;

    let res_header_wrap = HeaderWrap2::new(res.headers());

    let mut client_resp =
        HttpResponse::build(actix_web::http::StatusCode::from_u16(res.status().as_u16()).unwrap());

    let mut is_stream = false;
    for (header_name, header_value) in res
        .headers()
        .iter()
        .filter(|(h, _)| !h.as_str().eq_ignore_ascii_case("connection"))
    {
        if header_name
            .as_str()
            .eq_ignore_ascii_case("Transfer-Encoding")
            && header_value.as_ref().eq_ignore_ascii_case(b"chunked")
        {
            is_stream = true;
        }
        client_resp.insert_header((
            actix_web::http::header::HeaderName::from_bytes(header_name.as_ref()).unwrap(),
            actix_web::http::header::HeaderValue::from_bytes(header_value.as_ref()).unwrap(),
        ));
    }

    log::info!(
        "[req_{}] resp is stream:{},status:{}\n{}",
        request_id,
        is_stream,
        res.status(),
        &res_header_wrap
    );
    let save_requests = **save_all_requests;
    if is_stream {
        let (tx, rx) = tokio::sync::mpsc::channel(5);
        let r_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let request_id_copy = request_id;
        let date_copy = date.clone();
        let time_copy = time.clone();
        actix_web::rt::spawn(async move {
            let mut resp_steam = res.bytes_stream();
            let mut response_body = Vec::<u8>::new();
            while let Some(chunk) = resp_steam.next().await {
                if let Ok(v) = chunk.as_ref() {
                    let chunk_bytes = v.as_ref();
                    log::info!(
                        "[req_{}] resp chunk:\n\t{}",
                        request_id_copy,
                        String::from_utf8_lossy(chunk_bytes)
                    );
                    if save_requests {
                        response_body.extend_from_slice(chunk_bytes);
                    }
                } else {
                    break;
                }
                match tx.send(chunk).await {
                    Ok(_) => {}
                    Err(e) => {
                        log::warn!(
                            "[req_{}] sent chunk to stream is error:{}",
                            request_id_copy,
                            e.to_string()
                        );
                        break;
                    }
                }
            }
            if save_requests && !response_body.is_empty() {
                if let Err(e) =
                    save_response_to_file(request_id_copy, &date_copy, &time_copy, &response_body)
                        .await
                {
                    log::error!("[req_{}] 保存响应内容失败: {}", request_id_copy, e);
                }
            }
        });
        Ok(client_resp.streaming(r_stream))
    } else {
        let data = res.bytes().await.map_err(error::ErrorInternalServerError)?;
        log::info!(
            "[req_{}] resp body:\n\t{}",
            request_id,
            String::from_utf8_lossy(data.as_ref())
        );
        if save_requests && !data.is_empty() {
            if let Err(e) = save_response_to_file(request_id, &date, &time, &data).await {
                log::error!("[req_{}] 保存响应内容失败: {}", request_id, e);
            }
        }
        Ok(client_resp.body(data))
    }
}

#[derive(clap::Parser, Debug)]
struct CliArguments {
    listen_addr: String,
    listen_port: u16,
    forward_url: String,
    #[clap(long, short = 's', help = "是否保存所有请求内容到文件")]
    save_all_requests: bool,
}

#[actix_web::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    let args = CliArguments::parse();
    log::info!("args:{:?}", &args);

    let forward_url = Url::parse(&args.forward_url).expect("Invalid forward_url provided");

    log::info!(
        "starting HTTP server at http://{}:{}",
        &args.listen_addr,
        args.listen_port
    );

    log::info!("forwarding to {forward_url}");

    //let reqwest_client = reqwest::Client::default();
    let reqwest_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(reqwest_client.clone()))
            .app_data(web::Data::new(forward_url.clone()))
            .app_data(web::Data::new(args.save_all_requests))
            .wrap(middleware::Logger::default())
            .default_service(web::to(forward_reqwest))
    })
    .bind((args.listen_addr, args.listen_port))?
    .workers(2)
    .run()
    .await
}
