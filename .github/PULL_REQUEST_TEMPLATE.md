## 变更说明

请简要说明本次改动解决了什么问题、采用了什么方案。

## 关联信息

- 关联 Issue: <!-- 例如 #123 -->
- 影响范围: <!-- 例如 scanner / recovery / docs -->

## 测试结果

请粘贴或描述你执行的测试：

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

## 检查清单

- [ ] 已阅读 `CONTRIBUTING.md`
- [ ] 代码与注释已自检，未引入明显回归
- [ ] 如有 CLI/配置变更，已更新 `README.md` 或 `README.en.md`
- [ ] 不包含敏感数据与未授权取证内容
