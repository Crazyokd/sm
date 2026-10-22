# E2E 开发规则

先读 README.md、TEST_DEVELOPMENT_GUIDE.md、../../docs/testing/assert_rules.md 和 ../profiles/README.md。
所有新用例使用原生 pytest；fixture、常量和参数化的职责以开发规范为准，禁止恢复 CASE/run 注册器或自定义结果模型。
双入口差异、单入口例外与独立性以 TEST_DEVELOPMENT_GUIDE.md 为准。
公共协议实现来自锁定 wheel；修复应回到来源资产仓，禁止在 EasyGo 复制或猴子补丁协议实现。
框架变更验证 ../runtime/tests；协议变更验证来源包测试；业务变更跑对应真实仿真。
旧 inttest 门禁只能在等价覆盖迁移 MR 中替换。
