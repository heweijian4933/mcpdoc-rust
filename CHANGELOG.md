# Changelog

本项目变更记录遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 格式,
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 待办

- SSE 传输支持
- 索引版本检测(避免文档未变化时重建)
- search_docs 按源过滤

## [0.1.0] - 2026-08-07

### 新增

- **核心功能**:基于 rmcp 0.2.1 的 MCP 文档服务器
  - `list_doc_sources` 工具:列出所有文档源(名称 + URL/路径 + 描述)
  - `fetch_docs` 工具:获取文档内容,HTML→Markdown,支持远程 HTTP 和本地文件
  - `search_docs` 工具:BM25 全文搜索,title 字段加权 2x
- **单 exe 挂载多个 llms.txt**:通过 `--urls`/`--yaml`/`--json` 配置多个文档源
- **本地文档支持**:绝对路径 + 相对路径(相对于 llms.txt 所在目录,带安全限制)
- **BM25 + header 索引优化**:tantivy 0.22 全文搜索引擎
  - 懒加载:首次 search_docs 调用时构建
  - 持久化缓存:索引保存到磁盘,多进程共享只读加载
  - 内容增强:llms.txt 中链接的描述文本加入 content 字段
- **域名白名单**:llms.txt 所在域名自动加入,`--allowed-domains '*'` 允许全部
- **stdio 传输**:适配 Cursor/Claude Code/Claude Desktop 等主流 MCP 客户端
- **HTML→Markdown 转换**:支持标题、段落、链接、列表、代码块、表格等
- **meta refresh 重定向处理**:`--follow-redirects` 时检测并跟随
- **CLI 参数**:对齐 Python mcpdoc,新增 `--index`/`--cache-dir`
- **提示词优化**:
  - "when to use" 触发条件(how to/what is/tutorial/持续犯错等)
  - search_docs 优先策略
  - 文档源自动提取描述(llms.txt 首行 `# 标题`)
- **离线构建**:vendor 模式,498 个依赖包
- **跨平台**:Windows/Linux/macOS

### 技术栈

- rmcp 0.2.1(MCP 协议)
- tantivy 0.22(BM25 全文搜索)
- reqwest 0.12(HTTP 客户端)
- scraper 0.20(HTML 解析)
- clap 4.5(CLI)
- tokio(异步运行时)
