# 目录作用与重要性分级

生成日期：2026-06-04  
范围：当前仓库目录结构。  
说明：本文覆盖项目有效目录；`.git`、`node_modules`、`sdk/typescript/node_modules` 等依赖或工具生成目录不作为源码架构单元逐项展开，仅在非源码目录表中标注。

## 分级标准

| 等级 | 含义 | 判断标准 |
| --- | --- | --- |
| P0 | 核心路径 | 直接影响 Codex 主运行链路、对外 API、核心协议、会话执行或主要交互入口 |
| P1 | 重要支撑 | 支撑 P0 模块运行，涉及配置、状态、认证、执行、插件、模型、沙箱、构建或测试 |
| P2 | 辅助能力 | 局部功能、工具库、样例、实验能力、专项集成 |
| P3 | 工程材料 | 文档、开发辅助、CI、编辑器配置、依赖缓存或非源码目录 |

## 顶层目录

| 目录 | 等级 | 作用 | 备注 |
| --- | --- | --- | --- |
| `codex-rs/` | P0 | Rust 主工作区，承载 Codex CLI、TUI、app-server、core、protocol、执行、配置、插件等主要实现 | 当前产品主体 |
| `codex-cli/` | P1 | npm 包装入口，提供 `@openai/codex` 的 `bin/codex.js` 和安装相关脚本 | 分发层，不是主要业务实现 |
| `sdk/` | P1 | Python、TypeScript SDK 和 Python runtime 包装 | 面向外部集成和 SDK 用户 |
| `docs/` | P2 | 顶层项目文档 | 按项目规则，不应新增通用产品文档到此目录 |
| `scripts/` | P2 | 顶层维护和打包脚本 | 具体用途需按脚本读取确认 |
| `tools/` | P2 | 顶层工具目录，例如 argument comment lint | 与 `codex-rs/tools` 不同 |
| `third_party/` | P2 | 第三方源码或构建依赖，例如 V8 相关内容 | 供应链和构建相关 |
| `patches/` | P2 | 补丁材料 | 具体作用依赖构建或发布流程 |
| `dev_harness/` | P2 | 本地架构、设计、开发、测试、评估和工程辅助文档 | 当前新增文档所在区域 |
| `.github/` | P3 | GitHub issue template、actions、workflow、脚本 | CI 和仓库自动化 |
| `.devcontainer/` | P3 | Dev container 配置 | 本地/云开发环境 |
| `.vscode/` | P3 | VS Code 工作区配置 | 编辑器辅助 |
| `.codex/` | P3 | 本地 Codex 环境和技能配置 | 仓库内 Codex 辅助配置 |
| `.git/` | P3 | Git 元数据 | 非源码目录 |
| `node_modules/` | P3 | Node 依赖安装目录 | 非源码目录，通常不纳入架构分析 |

## `dev_harness/` 子目录

| 目录 | 等级 | 作用 | 备注 |
| --- | --- | --- | --- |
| `dev_harness/arch/` | P2 | 架构分析、架构决策草案、系统边界说明 | 当前文档位于 `arch/global/` |
| `dev_harness/arch/global/` | P2 | 跨版本全局架构基线 | 当前全局架构文档目录 |
| `dev_harness/arch/versions/` | P2 | 版本作用域架构材料 | 分支创建后按版本目录组织 |
| `dev_harness/arch/archive/` | P3 | 已归档版本架构材料 | 手动归档 |
| `dev_harness/design/` | P2 | 产品设计、交互设计、用户工作流分析 | 非实现代码 |
| `dev_harness/develop/` | P2 | 开发计划、实现记录、权衡分析 | 版本开发过程材料 |
| `dev_harness/engineering/` | P2 | 可复用工程标准、脚本和技术检查 | 包含分支命名规则 |
| `dev_harness/eval/` | P2 | 评估计划、数据集、结果和质量标准 | 用于质量度量 |
| `dev_harness/skills/` | P2 | Agent skill 草案和规范 | 工作流复用 |
| `dev_harness/test/` | P2 | 测试计划、测试用例、验证记录 | 与源码测试不同 |

## `codex-rs/` 核心目录

| 目录 | 等级 | 作用 | 备注 |
| --- | --- | --- | --- |
| `codex-rs/cli/` | P0 | `codex` 统一 CLI 二进制入口，分发 TUI、exec、app-server、MCP、plugin、sandbox 等子命令 | 命令入口层 |
| `codex-rs/core/` | P0 | 核心业务编排，包含 thread、session、task、tool、context、MCP、sandbox、rollout、guardian 等 | 当前职责最集中区域 |
| `codex-rs/tui/` | P0 | 终端 UI 主实现，包含 app、chatwidget、bottom pane、history rendering、onboarding 等 | 主要交互前端 |
| `codex-rs/app-server/` | P0 | JSON-RPC app-server，连接桌面端/IDE/远程客户端与 core thread/session | 主要服务端入口 |
| `codex-rs/app-server-protocol/` | P0 | app-server v1/v2 JSON-RPC 协议、schema 和 TypeScript 类型导出 | 外部 API 合约 |
| `codex-rs/protocol/` | P0 | core 内部协议模型，包含 `Op`、`Event`、`EventMsg`、配置类型、权限、模型项 | 内部事件边界 |
| `codex-rs/exec/` | P0 | 非交互 `codex exec` 和 review 工作流实现 | 自动化入口 |
| `codex-rs/mcp-server/` | P0 | 将 Codex 暴露为 MCP server | 外部 MCP client 可调用 |
| `codex-rs/codex-mcp/` | P0 | Codex 作为 MCP client 的连接、工具、资源和服务器管理能力 | core 依赖该能力 |
| `codex-rs/app-server-client/` | P1 | app-server client，用于 TUI/exec 连接 in-process 或 remote app-server | 交互层适配 |
| `codex-rs/app-server-transport/` | P1 | app-server transport 抽象和 stdio/unix/ws 等连接支撑 | 通信基础设施 |
| `codex-rs/app-server-daemon/` | P1 | app-server daemon 生命周期管理 | 桌面/远程控制相关 |
| `codex-rs/app-server-test-client/` | P2 | app-server 测试客户端 | 测试辅助 |
| `codex-rs/config/` | P1 | 配置加载、profile、managed config、权限相关配置解析 | 影响启动和运行时行为 |
| `codex-rs/state/` | P1 | 状态数据库、日志、goal/memory migration | 持久化支撑 |
| `codex-rs/rollout/` | P1 | session rollout、线程历史、归档和恢复相关持久化 | resume/fork 关键路径 |
| `codex-rs/thread-store/` | P1 | thread store 抽象和本地线程存储 | thread lifecycle 支撑 |
| `codex-rs/exec-server/` | P1 | 命令、PTY、文件系统、HTTP/WebSocket relay 等执行服务 | 工具执行关键路径 |
| `codex-rs/sandboxing/` | P1 | 沙箱策略与兼容层 | 执行安全关键路径 |
| `codex-rs/linux-sandbox/` | P1 | Linux 沙箱实现 | 平台执行安全 |
| `codex-rs/windows-sandbox-rs/` | P1 | Windows 沙箱实现 | 平台执行安全 |
| `codex-rs/bwrap/` | P1 | Bubblewrap 沙箱相关封装 | Linux 沙箱相关 |
| `codex-rs/execpolicy/` | P1 | exec policy 检查与规则 | 命令执行约束 |
| `codex-rs/execpolicy-legacy/` | P2 | 旧 execpolicy 实现或兼容层 | 迁移/兼容相关 |
| `codex-rs/tools/` | P1 | 工具定义、工具执行抽象和测试 | 与 core tool runtime 协作 |
| `codex-rs/shell-command/` | P1 | shell 命令模型和解析支撑 | 执行链路基础类型 |
| `codex-rs/shell-escalation/` | P1 | Unix shell 权限提升相关能力 | 平台执行相关 |
| `codex-rs/plugin/` | P1 | 插件系统基础能力 | 扩展能力支撑 |
| `codex-rs/core-plugins/` | P1 | core 内置插件管理和注入 | core 扩展能力 |
| `codex-rs/core-skills/` | P1 | core 内置技能相关能力 | 模型上下文与工具辅助 |
| `codex-rs/skills/` | P1 | 技能发现、加载或通用 skill 支撑 | 与 core skills 区分 |
| `codex-rs/ext/` | P1 | 扩展 crate 集合，包括 goal、guardian、image-generation、memories、web-search 等 | 见扩展目录表 |
| `codex-rs/model-provider/` | P1 | 模型 provider 抽象与创建 | 模型选择和请求路径 |
| `codex-rs/model-provider-info/` | P1 | 模型 provider 元信息 | 配置和模型目录支撑 |
| `codex-rs/models-manager/` | P1 | 模型列表、刷新和缓存管理 | TUI/app-server/core 均可能使用 |
| `codex-rs/codex-api/` | P1 | Codex API 相关类型或客户端支撑 | 后端 API 集成 |
| `codex-rs/codex-client/` | P1 | Codex 客户端能力 | 与后端/API 交互相关 |
| `codex-rs/backend-client/` | P1 | 后端 client | 服务端集成 |
| `codex-rs/codex-backend-openapi-models/` | P2 | 后端 OpenAPI 生成模型 | API 类型支撑 |
| `codex-rs/login/` | P1 | 认证、登录、token 和账户限制 | 启动和请求权限关键路径 |
| `codex-rs/secrets/` | P1 | 密钥相关存储或处理 | 认证安全支撑 |
| `codex-rs/keyring-store/` | P1 | 系统 keyring 存储适配 | 凭据存储 |
| `codex-rs/aws-auth/` | P2 | AWS 认证相关能力 | 专项集成 |
| `codex-rs/chatgpt/` | P1 | ChatGPT 相关工作区设置或命令集成 | 账户和分发集成 |
| `codex-rs/cloud-config/` | P1 | 云端配置 bundle 加载 | TUI/app-server 启动配置 |
| `codex-rs/cloud-tasks/` | P2 | Codex Cloud task CLI/工作流 | 实验或云任务入口 |
| `codex-rs/cloud-tasks-client/` | P2 | Cloud tasks client | 云任务集成 |
| `codex-rs/cloud-tasks-mock-client/` | P2 | Cloud tasks mock client | 测试辅助 |
| `codex-rs/connectors/` | P1 | connector 相关类型和能力 | 工具/插件/上下文集成 |
| `codex-rs/context-fragments/` | P1 | 上下文片段类型与渲染支撑 | model context 边界 |
| `codex-rs/prompts/` | P1 | prompt 模板和内置 prompt | 模型行为基础输入 |
| `codex-rs/collaboration-mode-templates/` | P1 | collaboration mode 模板 | 模式化 agent 行为配置 |
| `codex-rs/code-mode/` | P2 | code mode 相关能力 | 专项模式 |
| `codex-rs/features/` | P1 | feature flag 定义和阶段 | 配置与实验功能控制 |
| `codex-rs/hooks/` | P1 | hook schema 和运行支撑 | 工具前后置控制 |
| `codex-rs/guardian/` | P1 | 通过 `ext/guardian` 提供，见扩展目录 | 若存在同名依赖以实际 workspace 为准 |
| `codex-rs/analytics/` | P1 | analytics 事件和上报模型 | 运行观测 |
| `codex-rs/otel/` | P1 | OpenTelemetry 初始化和指标 | 可观测性 |
| `codex-rs/feedback/` | P2 | 用户反馈收集或上传 | 支撑功能 |
| `codex-rs/network-proxy/` | P1 | 网络代理策略与运行支持 | 权限/网络访问控制 |
| `codex-rs/responses-api-proxy/` | P2 | Responses API 代理 | 调试或内部代理能力 |
| `codex-rs/response-debug-context/` | P2 | Response debug context | 调试辅助 |
| `codex-rs/realtime-webrtc/` | P2 | Realtime WebRTC 支撑 | 语音/实时交互相关 |
| `codex-rs/lmstudio/` | P2 | LM Studio provider 或集成 | 本地模型集成 |
| `codex-rs/ollama/` | P2 | Ollama provider 或集成 | 本地模型集成 |
| `codex-rs/agent-identity/` | P2 | agent 身份信息 | 多 agent 或标识支撑 |
| `codex-rs/agent-graph-store/` | P2 | agent graph store | 多 agent/图状态实验支撑 |
| `codex-rs/external-agent-migration/` | P2 | 外部 agent 配置迁移 | app-server/桌面兼容 |
| `codex-rs/external-agent-sessions/` | P2 | 外部 agent session 管理 | 外部 agent 集成 |
| `codex-rs/memories/` | P1 | memory read/write crate 集合 | 见 memory 目录表 |
| `codex-rs/message-history/` | P1 | 消息历史管理 | TUI/session 历史支撑 |
| `codex-rs/file-search/` | P1 | 文件搜索能力 | TUI/app-server 搜索 |
| `codex-rs/file-system/` | P1 | 文件系统抽象 | exec-server 和权限路径 |
| `codex-rs/file-watcher/` | P1 | 文件监听 | app-server/skills/config 更新支撑 |
| `codex-rs/git-utils/` | P1 | Git 信息、diff、状态辅助 | 线程元数据和 UI 展示 |
| `codex-rs/install-context/` | P2 | 安装上下文识别 | 启动/分发辅助 |
| `codex-rs/process-hardening/` | P1 | 进程安全加固 | 执行安全 |
| `codex-rs/terminal-detection/` | P2 | 终端识别 | TUI 行为适配 |
| `codex-rs/stdio-to-uds/` | P2 | stdio 到 Unix domain socket 转发 | app-server/remote control 辅助 |
| `codex-rs/uds/` | P2 | Unix domain socket 支撑 | transport 支撑 |
| `codex-rs/arg0/` | P2 | 通过 argv[0] 分发路径 | CLI/app-server 启动辅助 |
| `codex-rs/apply-patch/` | P1 | apply_patch 工具实现 | 代码修改工具关键能力 |
| `codex-rs/ansi-escape/` | P2 | ANSI escape 处理 | TUI 渲染支撑 |
| `codex-rs/async-utils/` | P2 | 异步工具函数 | 通用支撑库 |
| `codex-rs/core-api/` | P2 | core API 边界或适配 | 具体职责需按源码进一步确认 |
| `codex-rs/codex-experimental-api-macros/` | P1 | experimental API derive/proc macro | app-server protocol 实验字段控制 |
| `codex-rs/rmcp-client/` | P1 | rmcp client 封装 | MCP client 支撑 |
| `codex-rs/rollout-trace/` | P2 | rollout trace 回放或调试 | 调试和分析 |
| `codex-rs/thread-manager-sample/` | P2 | ThreadManager 样例 | 示例/验证 |
| `codex-rs/test-binary-support/` | P2 | 测试二进制定位和支持 | 测试基础设施 |
| `codex-rs/utils/` | P1 | 通用工具 crate 集合 | 见 utils 表 |
| `codex-rs/vendor/` | P2 | vendored 第三方代码，例如 bubblewrap | 构建/平台依赖 |
| `codex-rs/v8-poc/` | P3 | V8 proof-of-concept | 实验目录 |
| `codex-rs/docs/` | P3 | Rust 工作区内部文档 | 不等同顶层 `docs/` |
| `codex-rs/scripts/` | P3 | Rust 工作区脚本 | 构建/维护辅助 |
| `codex-rs/.cargo/` | P3 | Cargo 配置 | 构建配置 |
| `codex-rs/.config/` | P3 | 工作区配置 | 工程配置 |
| `codex-rs/.github/` | P3 | Rust 工作区 GitHub 配置 | CI/工作流辅助 |

## `codex-rs/ext/` 扩展目录

| 目录 | 等级 | 作用 | 备注 |
| --- | --- | --- | --- |
| `codex-rs/ext/extension-api/` | P1 | 扩展 API 抽象和注册机制 | app-server/core 扩展边界 |
| `codex-rs/ext/goal/` | P1 | goal 扩展能力 | goal runtime 相关 |
| `codex-rs/ext/guardian/` | P1 | guardian 审核/策略扩展 | 安全和审批相关 |
| `codex-rs/ext/image-generation/` | P2 | 图像生成扩展 | 工具/多模态能力 |
| `codex-rs/ext/memories/` | P1 | memory 扩展能力 | 模型上下文和用户记忆 |
| `codex-rs/ext/skills/` | P1 | skills 扩展能力 | 技能加载/暴露 |
| `codex-rs/ext/web-search/` | P2 | web search 扩展能力 | 外部信息检索 |

## `codex-rs/memories/` 子目录

| 目录 | 等级 | 作用 | 备注 |
| --- | --- | --- | --- |
| `codex-rs/memories/read/` | P1 | memory 读取能力 | session/context 输入相关 |
| `codex-rs/memories/write/` | P1 | memory 写入能力 | 记忆持久化相关 |

## `codex-rs/utils/` 工具 crate

| 目录 | 等级 | 作用 | 备注 |
| --- | --- | --- | --- |
| `codex-rs/utils/absolute-path/` | P1 | 绝对路径类型和路径解析 | 多模块共享 |
| `codex-rs/utils/path-utils/` | P1 | 路径工具函数 | 文件/配置/线程路径支撑 |
| `codex-rs/utils/approval-presets/` | P2 | approval preset 支撑 | TUI/config 相关 |
| `codex-rs/utils/cache/` | P2 | 缓存工具 | 通用支撑 |
| `codex-rs/utils/cargo-bin/` | P2 | 测试中定位 workspace 二进制 | 测试基础设施 |
| `codex-rs/utils/cli/` | P1 | CLI 共享选项和工具 | 多入口共享 |
| `codex-rs/utils/elapsed/` | P2 | elapsed time 工具 | UI/日志支撑 |
| `codex-rs/utils/fuzzy-match/` | P2 | 模糊匹配 | 搜索/UI 支撑 |
| `codex-rs/utils/home-dir/` | P1 | home/codex home 解析 | 配置和状态路径 |
| `codex-rs/utils/image/` | P2 | 图像处理工具 | 多模态支撑 |
| `codex-rs/utils/json-to-toml/` | P2 | JSON 到 TOML 转换 | 配置写入辅助 |
| `codex-rs/utils/oss/` | P2 | OSS provider 默认行为或校验 | 本地模型/开源 provider |
| `codex-rs/utils/output-truncation/` | P2 | 输出截断工具 | exec/tool 输出控制 |
| `codex-rs/utils/plugins/` | P2 | 插件工具函数 | 插件系统支撑 |
| `codex-rs/utils/pty/` | P1 | PTY 工具 | exec-server/TUI 命令执行 |
| `codex-rs/utils/readiness/` | P2 | readiness 检查 | 服务启动辅助 |
| `codex-rs/utils/rustls-provider/` | P2 | rustls provider 初始化 | 网络客户端支撑 |
| `codex-rs/utils/sandbox-summary/` | P2 | 沙箱策略摘要 | UI/CLI 展示 |
| `codex-rs/utils/sleep-inhibitor/` | P2 | 防止系统休眠 | 长任务运行辅助 |
| `codex-rs/utils/stream-parser/` | P2 | 流解析工具 | 模型/网络流处理 |
| `codex-rs/utils/string/` | P2 | 字符串工具 | 通用支撑 |
| `codex-rs/utils/template/` | P2 | 模板工具 | prompt/config 等文本生成 |

## SDK 目录

| 目录 | 等级 | 作用 | 备注 |
| --- | --- | --- | --- |
| `sdk/python/` | P1 | Python SDK 源码、示例、测试和文档 | 外部集成路径 |
| `sdk/python/src/openai_codex/` | P1 | Python SDK 包源码 | SDK 主实现 |
| `sdk/python/examples/` | P2 | Python SDK 示例 | 使用场景覆盖 |
| `sdk/python/tests/` | P2 | Python SDK 测试 | SDK 验证 |
| `sdk/python/docs/` | P3 | Python SDK 文档 | SDK 说明 |
| `sdk/python/notebooks/` | P3 | Python notebook 示例或实验材料 | 辅助材料 |
| `sdk/python/scripts/` | P3 | Python SDK 脚本 | 维护辅助 |
| `sdk/python-runtime/` | P1 | Python runtime 包装，包含 `codex_cli_bin` | SDK/runtime 分发支撑 |
| `sdk/typescript/` | P1 | TypeScript SDK 源码、样例和测试 | 外部集成路径 |
| `sdk/typescript/src/` | P1 | TypeScript SDK 主源码 | SDK 主实现 |
| `sdk/typescript/samples/` | P2 | TypeScript SDK 示例 | 使用场景覆盖 |
| `sdk/typescript/tests/` | P2 | TypeScript SDK 测试 | SDK 验证 |
| `sdk/typescript/dist/` | P3 | TypeScript 构建输出 | 生成目录 |
| `sdk/typescript/node_modules/` | P3 | TypeScript SDK 局部依赖 | 非源码目录 |

## `codex-cli/` npm 包装目录

| 目录 | 等级 | 作用 | 备注 |
| --- | --- | --- | --- |
| `codex-cli/bin/` | P1 | npm 包可执行入口 | 分发入口 |
| `codex-cli/scripts/` | P2 | npm 包相关脚本 | 安装/维护辅助 |

## 顶层工程与依赖目录

| 目录 | 等级 | 作用 | 备注 |
| --- | --- | --- | --- |
| `.github/actions/` | P3 | GitHub Actions 复用 action | CI 支撑 |
| `.github/workflows/` | P3 | GitHub workflow | CI/CD |
| `.github/scripts/` | P3 | GitHub workflow 脚本 | CI 辅助 |
| `.github/ISSUE_TEMPLATE/` | P3 | GitHub issue 模板 | 协作流程 |
| `.github/codex/` | P3 | GitHub 中 Codex 相关配置或脚本 | 需按具体文件确认 |
| `.devcontainer/codex-install/` | P3 | devcontainer 内 Codex 安装配置 | 开发环境 |
| `.codex/environments/` | P3 | Codex 环境配置 | 本地辅助 |
| `.codex/skills/` | P3 | Codex 技能配置 | 本地辅助 |
| `node_modules/` | P3 | 顶层 Node 依赖 | 非源码目录 |
| `.git/` | P3 | Git 元数据 | 非源码目录 |

## 重要性分布结论

| 分组 | 目录集合 | 架构含义 |
| --- | --- | --- |
| P0 主链路 | `codex-rs/cli`、`core`、`tui`、`app-server`、`app-server-protocol`、`protocol`、`exec`、`mcp-server`、`codex-mcp` | 决定 Codex 本地代理的主要入口、协议、会话执行和客户端交互 |
| P1 支撑链路 | 配置、状态、rollout、thread-store、exec-server、sandboxing、models、login、plugins、skills、tools、SDK | 影响主链路正确性、安全性、可扩展性和外部集成 |
| P2 专项能力 | cloud tasks、本地模型 provider、feedback、realtime、file search、实验/样例、顶层 tools/scripts | 局部功能或专项集成，变更影响通常可限定 |
| P3 工程材料 | docs、devcontainer、GitHub workflow、编辑器配置、生成目录、依赖目录 | 支撑工程流程，不直接构成运行时业务逻辑 |

## 使用建议

| 场景 | 优先查看目录 | 原因 |
| --- | --- | --- |
| 修改 agent/session 行为 | `codex-rs/core/`、`codex-rs/protocol/`、`codex-rs/core/tests/` | 行为源头和内部事件边界在 core/protocol |
| 修改 app-server API | `codex-rs/app-server-protocol/`、`codex-rs/app-server/` | 先协议类型，后 request processor |
| 修改 TUI 展示或交互 | `codex-rs/tui/`、`codex-rs/app-server-client/`、`codex-rs/app-server-protocol/` | TUI 应以协议事件和请求为边界 |
| 修改命令执行或权限 | `codex-rs/core/`、`codex-rs/exec-server/`、`codex-rs/sandboxing/`、`codex-rs/protocol/permissions.rs`、`codex-rs/config/` | 权限和执行跨多个层级 |
| 修改模型或 provider | `codex-rs/model-provider/`、`codex-rs/model-provider-info/`、`codex-rs/models-manager/`、`codex-rs/core/` | 模型选择和请求路径共享 |
| 修改插件、技能或 MCP | `codex-rs/plugin/`、`codex-rs/core-plugins/`、`codex-rs/skills/`、`codex-rs/codex-mcp/`、`codex-rs/core/src/tools/` | 扩展能力和工具生命周期相关 |
| 修改 SDK | `sdk/python/` 或 `sdk/typescript/` | 与 Rust 主工作区相对独立，但依赖 API 合约 |
