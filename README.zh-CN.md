# Option Workstation

一个以可验证性为优先的期权历史回放、波动率分析与受保护模拟交易工作台。

[English README](README.md) · [新手指南](frontend/public/guide.html) ·
[系统架构](docs/ARCHITECTURE.md) · [数据规范](docs/DATA_SOURCES.md) ·
[安全策略](SECURITY.md)

![Option Workstation 实时分析界面](frontend/public/guide/workbench-overview.png)

> [!WARNING]
> 本项目仍处于 1.0 之前的研究阶段，不构成投资建议、下单建议或对行情质量、
> 成交结果、盈利和亏损的保证。项目不支持实盘账户下单。模拟下单默认关闭，
> 且即使开启，仍然存在多腿拆单、部分成交和滑点风险。

## 项目解决什么问题

很多期权工具会把延迟行情、模型推导值和真实可成交价格混在一起。
Option Workstation 刻意把这些层次拆开并显示出来：

- 历史模式直接读取本地 ThetaData Parquet 分区，按时间点回放；
- 实时模式使用 Longbridge 官方 Rust SDK，并显示行情新鲜度；
- Bid/Ask 可用性、报价年龄和 OI 元数据覆盖率不会被隐藏；
- BSM、SVI、Greeks、GEX、波动率上下文和曲面由 Rust 服务统一计算；
- 组合分析按可执行 NBBO 边计价，而不是只展示理想化中间价；
- 关键截面可以写入追加式 JSONL 哈希链，便于复盘与追责；
- 模拟下单由多个彼此独立的服务端门禁保护。

它首先是一个研究与证伪工具。图形看起来平滑，不代表曲面无套利；没有足够 OI
覆盖时，系统会保留空值，而不是补造 PCR、GEX、墙位或 Gamma Flip。

## 核心能力

| 模块 | 当前能力 | 必须知道的边界 |
| --- | --- | --- |
| 历史回放 | 多标的分钟线、期权链、到期日与同步步进 | 需要用户自行取得合法授权的数据 |
| 实时分析 | Longbridge 行情/深度订阅与本地 WebSocket | 受账号权限和供应商限频约束 |
| 波动率 | BSM IV/Greeks、同 DTE IV 历史、RV、VRP、Expected Move | BSM 对美式期权只是近似 |
| 微笑与曲面 | Call/Put 微笑、SVI、残差、期限结构和约束曲面 | 研究投影，不是严格无套利证明 |
| 暴露 | GEX、Vanna、Charm、墙位与 Gamma Flip | Dealer 符号是模型假设 |
| 组合风险 | 多腿可成交计价、到期损益与 Spot/IV/时间情景矩阵 | 多腿并非交易所原子撮合 |
| 审计 | 凭证字段拒绝、JSONL 哈希链 | 本地完整性辅助，不是第三方公证 |
| 交易 | 账户与订单监控、严格受控的模拟限价单 | 明确拒绝实盘账户下单 |

`backend/` 只保留为 Python 迁移对照。正式应用由 `rust-backend/` 提供服务。

## 快速启动

### 环境要求

- Rust stable，并安装 `rustfmt` 与 `clippy`
- Node.js 22 与 npm
- Python 3.11+，仅在运行旧 Python 对照测试时需要
- 历史模式需要用户自行准备合法授权的回放数据
- 实时模式可选 Longbridge OpenAPI 凭证

```bash
git clone <你的仓库或 fork 地址> OptionWorkstation
cd OptionWorkstation
cp .env.example .env
```

如果有历史数据，修改 `.env` 中的 `OPTION_WORKSTATION_DATA_ROOT`，然后：

```bash
make setup
make run
```

打开 <http://127.0.0.1:7311>。没有历史数据时服务仍可启动，但历史目录为空。

执行完整工程检查：

```bash
make check
make security
```

### Docker

```bash
cp .env.example .env
docker compose up --build
```

Compose 默认只绑定 `127.0.0.1:7311`，历史数据以只读方式挂载，审计记录保存在
独立 volume 中。

## 历史数据目录

```text
data/
├── underlying/
│   └── symbol=SPY/
│       └── date=2026-07-10/
│           └── ohlc.parquet
└── options/
    └── symbol=SPY/
        └── date=2026-07-10/
            └── expiration=2026-07-10/
                ├── quote_1m.parquet
                └── open_interest.parquet
```

字段、类型、时区、时间点规则和数据许可要求见
[docs/DATA_SOURCES.md](docs/DATA_SOURCES.md)。仓库会忽略所有 Parquet、Arrow
和 Feather 文件，不附带任何 ThetaData 或 Longbridge 数据。

## 实时连接

1. 进入工作台并切换到“实时”。
2. 打开连接面板。
3. 填入 Longbridge App Key、App Secret 和 Access Token。
4. 连接后选择标的与到期日，等待质量门禁通过。

凭证只发送给同源 Rust API，并由 SDK Context 保存在进程内存中。服务不会把凭证
返回浏览器、写进 localStorage、提交到审计记录或保存到仓库。断开连接或停止进程
后，内存会话即被清除。

不要直接把本服务暴露到公网。默认只监听本机回环地址；若确需网络访问，必须先补充
身份认证、TLS、Origin 限制和主机访问控制。

## 模拟下单门禁

必须同时满足以下条件，服务端才允许提交模拟订单：

1. Longbridge 返回模拟账户类型；
2. 服务端显式设置 `OPTION_WORKSTATION_PAPER_ORDER_EXECUTION=1`；
3. 当前组合预览仍然新鲜且具有可执行报价；
4. 用户输入完全一致的确认词 `PAPER`。

订单使用 RTH-only Day Limit，并带确定性的请求 ID。系统先提交买腿，再提交卖腿；
后续腿失败时会请求撤销前面已提交的腿。这不是交易所级别的原子多腿订单，因此仍有
部分成交、裸露风险和滑点。

## 环境变量

| 变量 | 默认值 | 作用 |
| --- | --- | --- |
| `OPTION_WORKSTATION_DATA_ROOT` | `./data` | 历史正股与期权分区 |
| `OPTION_WORKSTATION_HOST` | `127.0.0.1` | HTTP/WebSocket 监听地址 |
| `OPTION_WORKSTATION_PORT` | `7311` | 服务端口 |
| `OPTION_WORKSTATION_RISK_FREE_RATE` | `0.043` | BSM 无风险利率 |
| `OPTION_WORKSTATION_FRONTEND_DIST` | `./frontend/dist` | 前端构建目录 |
| `OPTION_WORKSTATION_AUDIT_PATH` | `~/.option-workstation/audit.jsonl` | 追加式审计记录 |
| `OPTION_WORKSTATION_PAPER_ORDER_EXECUTION` | 未设置 | 模拟下单服务端总开关 |
| `RUST_LOG` | `option_workstation=info,tower_http=info` | 日志过滤器 |

不要把 Longbridge 凭证写进 `.env`。本项目只通过本机同源连接面板接收凭证。

## 验证

```bash
./scripts/verify.sh http://127.0.0.1:7311
RUN_BROWSER_SMOKE=1 ./scripts/verify.sh http://127.0.0.1:7311
node scripts/live-switch-smoke.mjs http://127.0.0.1:7311
node scripts/guide-smoke.mjs http://127.0.0.1:7311
```

基础验证包含 Rust 格式检查、单元测试、Clippy 零警告和前端生产构建。浏览器测试
检查桌面/移动端溢出、WebGL 非空以及 3D 相机在数据刷新时不复位。所有自动化测试
都禁止调用订单变更接口。

## 模型边界

- 美股上市期权通常是美式期权，本地 BSM 是欧式近似。
- 约束总方差曲面会报告价格空间违规与调整数量，但不宣称严格无套利。
- Dealer GEX 符号不是可观测的真实库存。
- IV Rank/Percentile 只使用同 DTE 的历史 ATM IV，并显示样本数和最后日期。
- RV 只使用当前会话之前已经完成的前复权日线。
- 0DTE Expected Move 使用距离当日 16:00 ET 的精确剩余时间。
- 屏幕上的 NBBO 不保证成交，尤其是宽价差、陈旧报价和多腿组合。

更完整说明见 [docs/MODEL_LIMITATIONS.md](docs/MODEL_LIMITATIONS.md)。

## 开发规范

- 计算逻辑保持确定性，并由服务端统一负责。
- 数据来源、报价年龄、样本数和质量门禁必须可见。
- 历史回放必须保持 point-in-time 语义，禁止未来函数。
- 默认按买入 Ask、卖出 Bid 建模；其他假设必须单独标注。
- 禁止提交行情数据、凭证、审计账本、账户标识或包含这些内容的截图。
- 测试范围要与改动风险匹配。
- 不得为了演示方便削弱模拟下单门禁。

贡献前请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。开源发布前还需由维护者完成
[docs/RELEASE_CHECKLIST.md](docs/RELEASE_CHECKLIST.md) 中的 GitHub 仓库设置，
包括私密漏洞报告、分支保护和必需检查。

当前锁定的 Longbridge SDK 需要仓库内的 `longbridge-oauth` 安全补丁，以将
OAuth 依赖升级到 `oauth2` 5.0。补丁源码、上游许可证与变更范围均保存在
[`vendor/longbridge-oauth`](vendor/longbridge-oauth)；上游发布等效修复后应移除此补丁。

## 许可证

项目使用 [Apache License 2.0](LICENSE)。第三方组件保留其原许可证，详见
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

行情访问和再分发受各数据供应商协议单独约束。本仓库不会授予用户传播 ThetaData、
Longbridge、交易所或券商数据的权利。
