# 贡献指南

感谢你为 YanMiRestore 做贡献。

## 提交前约定

- 仅提交你有权提交的代码与文档。
- 不要提交任何真实敏感数据、取证样本或用户隐私文件。
- 不要实现绕过系统加密、绕过设备锁定等违法能力。

## 开发环境

- Rust stable（建议与 CI 同版本）
- 建议使用 `cargo` 原生命令完成构建与测试

## 本地检查

提交前请至少执行：

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

## 提交规范

- 建议使用 Conventional Commits：
  - `feat:`
  - `fix:`
  - `docs:`
  - `refactor:`
  - `test:`
  - `chore:`
- 提交应聚焦单一主题，避免“大杂烩”。
- 如果改动了 CLI 参数、默认行为或配置项，请同步更新 `README.md`/`README.en.md`。

## Pull Request 要求

创建 PR 时请说明：

- 改动背景与目标
- 核心实现思路
- 风险与兼容性影响
- 测试范围和结果

如涉及恢复流程变更，请附上最小可复现命令与示例输出。

## 问题反馈

- 功能缺陷请使用 Bug 模板
- 新功能建议请使用 Feature 模板
- 安全问题请走 `SECURITY.md` 指定流程，不要公开披露细节
