# AI Request Proxy

一个简单的 HTTP 请求转发代理服务器，专门用于调试和监控 AI API 请求。

## 功能特性

- **请求转发代理**：将客户端请求透明转发到指定的 AI 服务端点
- **流式响应支持**：完整支持 SSE 和流式响应
- **详细日志记录**：记录所有请求和响应的完整信息，包括 headers、body chunks
- **请求内容保存**：可选地将请求和响应保存到本地文件
- **结构化内容解析**：自动解析 OpenAI 格式的请求，包括工具调用（tool calls）
- **请求追踪**：为每个请求分配唯一 ID 便于追踪调试

## 安装

### 从源码编译

```bash
# 克隆仓库
git clone <repository-url>
cd ai-req-proxy

# 编译
cargo build --release

# 编译后的二进制文件位于 target/release/ai-req-proxy
```

## 使用方法

### 基本语法

```bash
ai-req-proxy <监听地址> <监听端口> <转发目标URL> [选项]
```

### 参数说明

- `监听地址`：代理服务器监听的网络地址，如 `0.0.0.0` 或 `127.0.0.1`
- `监听端口`：代理服务器监听的端口号
- `转发目标URL`：请求转发的目标 AI 服务地址
- `-s, --save-all-requests`：可选，启用后将保存所有请求和响应内容到文件

### 启动示例

```bash
# 基本启动（不保存请求）
ai-req-proxy 0.0.0.0 7080 https://open.bigmodel.cn

# 启用请求保存功能
ai-req-proxy 0.0.0.0 7080 https://open.bigmodel.cn -s

# 只监听本地
ai-req-proxy 127.0.0.1 8080 https://api.openai.com -s
```

### 使用方式

启动代理后，将 AI API 客户端的请求地址改为代理地址即可：

```bash
# 例如使用 curl 发送请求
curl http://127.0.0.1:7080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -d '{
    "model": "gpt-3.5-turbo",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

## 请求内容保存

启用 `-s` 选项后，请求和响应会按照以下结构保存到文件系统：

```
data/req/
└── 20240107/
    ├── 20240107_143022_000001.json           # 原始请求 JSON
    ├── 20240107_143022_000001.struct_req.txt  # 结构化请求文本
    └── 20240107_143022_000001.resp.json      # 响应 JSON
```

### 文件命名规则

- `{日期}_{时间}_{请求ID:06}.json` - 原始请求内容
- `{日期}_{时间}_{请求ID:06}.struct_req.txt` - 结构化解析后的请求（包含工具调用等详细信息）
- `{日期}_{时间}_{请求ID:06}.resp.json` - 响应内容

### 结构化内容格式

对于 OpenAI 格式的请求，会自动解析并格式化为易读的文本格式，包括：
- Tools 定义
- 各个消息的内容和角色
- Assistant 的工具调用详情

## 日志输出

程序会输出详细的日志信息，包括：

- `[req_N] 请求ID` - 每个请求的唯一标识
- 请求方法、URL、headers
- 请求体 chunks（流式传输时）
- 响应状态、headers
- 响应体 chunks
- 文件保存状态

## 技术栈

- Rust 2024 Edition
- actix-web - Web 框架
- reqwest - HTTP 客户端
- tokio - 异步运行时
- clap - 命令行参数解析

## License

见 LICENSE 文件
