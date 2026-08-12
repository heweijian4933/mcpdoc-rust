# 贡献指南

感谢你对 mcpdoc-rust 的兴趣!本文档介绍如何参与贡献。

## 开发环境

### 前置要求

- Rust 1.75+(推荐 stable 最新版)
- Git

### 本地开发

```bash
# 克隆仓库
git clone https://github.com/heweijian4933/mcpdoc-rust.git
cd mcpdoc-rust

# 构建
cargo build

# 运行测试
cargo test

# 运行 clippy 检查
cargo clippy -- -D warnings

# 检查格式
cargo fmt -- --check
```

### 离线构建

项目支持 vendor 模式离线构建。详见 [RUST_OFFLINE_SETUP.md](RUST_OFFLINE_SETUP.md)。

```bash
# 解压 vendor 包后
cargo build --offline --release
```

## 代码规范

### 格式化

```bash
# 自动格式化
cargo fmt

# 检查格式(不修改)
cargo fmt -- --check
```

### Lint

```bash
# 运行 clippy
cargo clippy -- -D warnings
```

### 测试

```bash
# 运行所有测试
cargo test

# 运行特定模块测试
cargo test -- config
cargo test -- index
cargo test -- html_to_md
```

## 提交规范

### Commit Message 格式

使用约定式提交(Conventional Commits):

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

类型:
- `feat`: 新功能
- `fix`: Bug 修复
- `docs`: 文档变更
- `style`: 代码格式(不影响功能)
- `refactor`: 重构
- `test`: 测试相关
- `chore`: 构建/工具变更

示例:
```
feat(index): 支持 BM25 索引持久化缓存
fix(domain): 修复 Windows 盘符路径误判为 name 前缀
docs(readme): 更新 CLI 参数说明
```

### 分支命名

- `feat/<feature-name>`: 新功能
- `fix/<bug-description>`: Bug 修复
- `docs/<topic>`: 文档更新

## Pull Request 流程

1. Fork 仓库并创建分支
2. 确保测试和 lint 通过:`cargo test && cargo clippy && cargo fmt -- --check`
3. 提交 PR,描述变更内容和动机
4. 等待 CI 检查通过
5. 代码评审

## 项目结构

```
src/
├── lib.rs           # 库入口
├── main.rs          # 二进制入口(CLI + 传输分发)
├── cli.rs           # clap CLI 定义
├── config.rs        # DocSource + 配置加载
├── server.rs        # MCP ServerHandler + 工具
├── fetch.rs         # 文档获取(HTTP/本地)
├── html_to_md.rs    # HTML→Markdown 转换
├── domain.rs        # URL 域名 + 白名单 + 路径
├── error.rs         # 错误类型
└── index/
    ├── mod.rs       # tantivy BM25 索引
    └── schema.rs    # 索引 schema
```

## 添加新功能

### 新增 MCP 工具

1. 在 `src/server.rs` 的 `#[tool_router]` impl 块中添加方法
2. 用 `#[tool(description = "...")]` 标注
3. 定义参数结构体(derive `JsonSchema + Deserialize`)
4. 添加测试

### 新增文档源格式

1. 在 `src/config.rs` 的 `parse_llms_txt` 中支持新格式
2. 添加单元测试

## 报告问题

提交 Issue 时请包含:
- 问题描述
- 复现步骤
- 预期行为 vs 实际行为
- 环境(OS、Rust 版本)
- 日志输出(如有)

## 许可证

提交的贡献将遵循 MIT 许可证。
