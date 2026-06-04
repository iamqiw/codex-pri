# Codex CLI 无状态服务可行性分析

## 1. 分析范围

本分析基于当前代码结构，评估以下目标的可行性：

- 将 Codex CLI/app-server 的业务状态迁移到云端持久化。
- 降低服务实例对进程内长期状态的依赖。
- 将长链接对话改造为单次请求对话。
- 支持几十到上百个云端 Codex 实例中任意节点基于线程标识 resume 后续请求。
- 将 Skill 权威源改为 NFS，并在服务实例本地缓存。
- 为 Skill 更新提供缓存失效机制。
- 提供严格只读 Runtime：App-Server 主机、远程 exec-server workspace、NFS Skill 源、检索挂载和执行临时目录不得被模型驱动能力写入。
- 将 command/exec 保留为只读脚本执行能力，强制路由到只读 exec-server，禁止回退到 App-Server 主机。
- 将 `apply_patch` 类文件修改能力替换为云端 PatchArtifact 输出。
- 不支持运行中 SSE 断线重新 attach；断线触发当前 running turn 取消或中断。
- 不启用 sub-agent 或多 agent 编排能力。
- 可评估 Claude Code 类似 print/single-run mode，即每次调用只执行一个 turn 并释放本次 session 对象；当前 Codex CLI 的 `-p` 已被 `--profile` 使用，具体参数名属于兼容性决策。

本文只做可行性判断，不给出最终技术方案。

## 2. 当前代码结构结论

### 2.1 服务入口已具备基础

当前 `codex app-server` 已提供 JSON-RPC 服务入口，并已有 `thread/start`、`thread/resume`、`thread/list`、`thread/read`、`turn/start`、`skills/list`、`skills/extraRoots/set` 等方法。协议定义集中在 `codex-rs/app-server-protocol/src/protocol/common.rs` 和 `codex-rs/app-server-protocol/src/protocol/v2/`。

结论：服务化入口可复用，不需要从零构建服务协议。但若引入 `task/*` 抽象，仍需要新增 v2 API 类型、schema 生成和兼容测试。

### 2.2 线程持久化已有抽象边界

`codex-rs/thread-store/src/store.rs` 定义了 `ThreadStore` trait，覆盖创建、恢复、追加 item、flush、shutdown、读取历史、读取线程、列表、搜索、元数据更新、归档和反归档。`codex-rs/core/src/thread_manager.rs` 通过 `thread_store_from_config` 在 `LocalThreadStore` 和 `InMemoryThreadStore` 间选择实现。

结论：云端线程/事件存储具备清晰接入点。新增 `CloudThreadStore` 或等价实现是可行路径。该边界能覆盖已完成线程、历史读取、列表、归档等场景。

### 2.3 运行态仍依赖进程内对象

`ThreadManagerState` 中持有 `threads: Arc<RwLock<HashMap<ThreadId, Arc<CodexThread>>>>`。`turn/start` 当前通过 `load_thread` 从 `ThreadManager` 获取已加载的 `CodexThread`，找不到即返回 `thread not found`。`ThreadState` 与 `ThreadWatchManager` 也维护连接、listener、pending interrupt、当前 turn summary、watch channel 等进程内状态。

结论：查询类 API 可较快云端化；运行中请求、活动 turn、取消、流式订阅继续保持实例绑定。当前目标不包含运行中任务跨实例接管、运行中 SSE 断线重新 attach、审批编排、PTY/MCP 会话恢复。单次请求模型要求 turn 结束后释放进程内线程对象，并保证下一次请求可由任意节点从云端状态重建上下文。

### 2.4 当前状态存储仍有本地文件和本地 SQLite 假设

当前历史记录仍大量使用 rollout JSONL。`Session::flush_rollout`、`ensure_rollout_materialized`、`persist_rollout_items` 仍是核心路径。`codex-rs/rollout` 处理本地 rollout 文件读写、压缩、物化。`codex-rs/state` 使用 SQLite 维护部分运行时元数据，例如 goal、thread metadata、backfill state。

结论：`ThreadStore` 抽象降低了迁移难度，但本地 rollout path 仍泄漏到多处 API 和测试中。云端化不是单点替换，需要先收敛 path-addressed 操作，并将 rollout path 降为 LocalThreadStore 私有细节。`state_db` 云端化属于本版本必做范围，需要从 SQLite 绑定改为云端数据库或状态服务边界。

### 2.5 Skill 加载已有可替换核心，但缓存模型不满足需求

Skill 逻辑集中在 `codex-rs/core-skills`：

- `SkillsManager` 按 cwd 或 config 缓存 `SkillLoadOutcome`。
- 缓存 key 当前主要由 root path、scope、plugin id、skill config rules 组成。
- `load_skills_from_roots` 从 `SkillRoot` 扫描 `SKILL.md`。
- `SkillRoot` 已携带 `ExecutorFileSystem`，理论上支持非本地文件系统来源。
- `SkillsWatcher` 当前用本地 file watcher 注册 skill root 变更，并在变更时执行 `skills_manager.clear_cache()`。
- `skills/list` 支持 `forceReload`，但没有版本指纹、TTL、NFS 权威源、本地物化缓存或请求级版本绑定。

结论：Skill 改造可行，且改造面相对集中。但当前缓存是进程内结果缓存，不是本地文件缓存；失效是全量清理，不是基于版本的精确失效。

### 2.6 严格只读 Runtime 与现有可写路径冲突

当前 app-server README 暴露了多类可写或高权限能力：

- `thread/shellCommand` 被标注为 unsandboxed full access。
- `command/exec` 与 `process/spawn` 可运行命令或进程。
- `fs/writeFile`、`fs/createDirectory`、`fs/remove`、`fs/copy` 可直接修改文件系统。
- `environment/add` 可注册远程 exec-server 环境。
- 默认 agent 指令仍要求使用 `apply_patch` 修改文件。

结论：严格只读 Runtime 不是单纯配置 read-only sandbox 即可完成。必须定义一个 Runtime capability profile，并在 app-server、core tool registry、exec-server、MCP、hooks、skill script 和 patch 路径统一执行。不满足这一点时，远程 exec-server workspace 或 App-Server 主机会变成可写状态节点，后续请求无法任意迁移。

### 2.7 `codex exec` 与单次执行模式的边界

当前 `codex exec` 是非交互入口。执行路径会启动 in-process app-server client，创建或恢复 thread，再调用 `turn/start` 并等待完成。独立进程退出后，本次 in-process app-server、`CodexThread`、listener 和 turn-scoped 对象会被释放。

结论：每 turn 使用独立进程的 print/single-run mode 可以减少进程内 session 驻留，是可行的过渡形态。但它不自动解决以下问题：

- 线程、事件、state_db 和 artifact 是否写入云端。
- Thread append 是否具备 writer lease 和 fencing。
- command/exec 是否只读且只走 exec-server。
- `apply_patch` 是否停止写入 workspace。
- 后续请求是否可由任意 App-Server 节点从云端状态恢复。

当前 Codex CLI 的 `-p` 已用于 `--profile`，因此若采用 Claude Code 类似 `-p` 表达 print/single-run mode，需要做 CLI 兼容决策。

## 3. 分项可行性

### 3.1 云端状态存储

可行性：中等。

有利条件：

- `ThreadStore` 已是存储中立接口。
- app-server 多数 thread read/list/search 路径已通过 `thread_store`。
- `StoredThreadHistory` 已不要求文件路径。
- `LoadThreadHistoryParams`、`AppendThreadItemsParams` 等类型已用 `ThreadId` 作为主索引。

主要阻碍：

- `ReadThreadByRolloutPathParams` 和多处 resume/fork 测试仍依赖 rollout path。
- `LocalThreadStore` 仍以 rollout JSONL 和 SQLite 组合为事实存储。
- core session 在 turn 完成、rollback、review、shell output 等路径主动 flush/materialize rollout。
- state_db 是 SQLite 绑定，不是云端数据库接口。
- 当前 `AppendThreadItemsParams` 仅表达 `thread_id` 和 `items`，未表达 writer lease、fencing token 或 append 幂等键。

建议分阶段：

1. 新增云端 `ThreadStore` 实现，仅覆盖 create/resume/append/flush/read/list/archive。
2. 将 app-server 查询路径切到 store-first，避免扫描本地 rollout。
3. 收敛 `read_thread_by_rollout_path` 为 local-only 兼容路径。
4. 为 Thread append 和 Request 状态写入引入 writer lease、fencing token 和 append 幂等键。
5. 将 state_db 访问抽象为 trait，再实现云端 DB 版本，并覆盖 goal、thread metadata、memory mode、backfill state 等现有 SQLite 职责。

### 3.2 单次请求与实例绑定运行模型

可行性：中等偏高，但需要收敛当前 long-lived thread 假设。

当前目标定义为“每次用户输入是一次独立请求；请求运行期间可以绑定实例；请求完成后后续请求必须能由任意实例 resume”。该目标允许运行期复用当前 `ThreadManager`、`CodexThread` 和 listener，但要求请求边界结束后不能依赖这些进程内对象。

如果采用 Claude Code 类似 print/single-run mode，并且每次 turn 由独立进程执行，则进程退出会自然释放本次 Session、in-process app-server、listener 和工具运行对象。这能降低 App-Server 长驻对象清理复杂度，但不能替代云端 ThreadStore、云端 state_db、事件持久化、writer lease 和严格只读执行边界。

明确非目标：

- 不做运行中任务跨实例接管。
- 不支持运行中 SSE 断线重新 attach。
- 当前 Runtime 设计不覆盖审批编排、租户和外围 Session 管理。
- 不做 command exec、PTY、shell session、MCP 会话跨实例恢复。
- 不启用 sub-agent 或多 agent 编排。

建议首版约束：

- 查询、历史、事件读取、归档、元数据更新无状态化。
- 每次 turn start 前从云端状态加载线程历史、配置快照和 state_db 元数据。
- 每次 turn 完成或中断后关闭或可回收进程内 `CodexThread`。
- 运行中请求路由到 owner 实例；后续新请求不得要求命中同一实例。
- 实例异常时，运行中请求进入 `interrupted` 或 `failed`，不做跨实例继续执行。
- 运行中 SSE 断线时，owner 实例取消或中断当前 turn，并写入终态；终态后事件可通过 cursor 读取。
- 若使用长驻 App-Server 而不是独立进程模式，必须显式 release request-scoped runtime；不能仅依赖协议 flag。

对当前代码的主要影响：

- `turn/start` 当前要求 `ThreadManager::get_thread` 返回已加载线程；需要新增“按线程标识从 store resume 后立即执行 turn”的路径。
- `thread/resume` 当前是显式 API；单次请求模式下 resume 应成为 `turn/start` 或新单次请求 API 的内部步骤。
- `ThreadManagerState.threads` 可以继续作为运行期 registry，但不得作为后续请求正确性的必要条件。
- `Session::record_initial_history`、rollout reconstruction、context manager 更新路径需要保证从云端历史重建结果等价。

### 3.3 Skill NFS 权威源

可行性：中等偏高。

有利条件：

- Skill root 解析集中在 `core-skills/src/loader.rs`。
- `SkillRoot` 已有 `file_system: Arc<dyn ExecutorFileSystem>` 字段。
- `skills/extraRoots/set` 已能在运行时加入额外 root。
- `skills/list` 已支持 `forceReload`。
- loader 已限制扫描深度和单 root 目录数量，适合控制 NFS 扫描风险。

主要阻碍：

- 当前 user/system/admin/plugin skill root 大多绑定 `LOCAL_FS`。
- NFS 只是挂载路径时可复用 `LOCAL_FS`，但无法表达权威源、缓存路径、版本指纹、TTL。
- watcher 对 NFS 可靠性不应作为唯一失效机制。
- 当前 `SkillMetadata` 不包含版本指纹。
- skill injection 读取 `path_to_skills_md` 内容，未记录请求使用的 skill 版本。

建议分阶段：

1. 在配置层引入 NFS skill root，先作为 admin/user root 参与扫描。
2. 新增 `SkillVersion` 或 `SkillFingerprint`，写入 `SkillMetadata` 或旁路索引。
3. 新增 `SkillCacheManager`，负责 NFS 到本地缓存的物化、哈希、TTL、原子替换。
4. loader 读取本地缓存路径，但保留 source path 和 authoritative fingerprint。
5. turn start 时将使用的 Skill 指纹写入线程和请求事件。

### 3.4 Skill 缓存失效

可行性：中等。

当前已有机制：

- `SkillsManager::clear_cache()` 可清除进程内加载结果。
- `SkillsWatcher` 可发送 `skills/changed` 通知。
- `skills/list.forceReload` 可绕过 cwd cache。

缺失能力：

- 缓存粒度不是按 skill/root/version。
- cache key 不包含内容指纹。
- 无 TTL。
- 无主动刷新 API。
- 无缓存完整性校验。
- 无并发刷新互斥。
- 无“运行中请求绑定旧版本、新请求加载新版本”的语义。

建议实现顺序：

1. 先实现目录级 fingerprint，作为 cache key 的一部分。
2. 再实现 TTL 兜底失效。
3. 再实现主动刷新 API。
4. 最后实现精确到 Skill 的增量刷新。

### 3.5 严格只读 Runtime

可行性：中等偏高，但前提是将“只读”定义为系统边界，而不是某个工具的局部权限。

有利条件：

- 权限 profile 和 sandbox policy 已经存在 read-only 语义。
- exec-server 已是进程和文件系统访问的独立边界，适合作为只读脚本执行环境。
- app-server 已有 `environment/add` 和 command execution 相关协议，可复用部分传输能力。
- MCP、tool registry、filesystem、process、patch 等能力已有相对明确的模块边界。

主要阻碍：

- 当前 app-server 仍暴露 `thread/shellCommand`、`process/spawn` 和 fs write/remove/copy 等可写能力。
- `command/exec` 当前需要明确禁止本地 fallback，否则 App-Server 主机仍可能被写入。
- 默认 agent 指令和部分工具链假设 `apply_patch` 会修改 workspace。
- hooks、skill script、MCP 工具可能绕过 shell/process 主路径写入文件。
- 如果 exec-server workspace 可写，它会成为有状态执行节点，破坏任意节点迁移前提。

建议首版约束：

1. 定义 `ReadOnlyRuntimeProfile`，作为云端 Runtime 的强制 capability profile。
2. 云端 Runtime 下不注册或拒绝 `fs/writeFile`、`fs/createDirectory`、`fs/remove`、`fs/copy`、`process/spawn` 写入型能力和 `thread/shellCommand`。
3. `command/exec` 仅允许路由到 read-only exec-server；缺失 read-only exec-server 时返回环境错误，不回退本机。
4. exec-server workspace、NFS Skill 源、检索挂载和执行临时目录均使用只读挂载或等价权限。
5. 将 `apply_patch` 替换为 PatchArtifact 输出，后续应用 patch 属于 Runtime 外部流程。
6. 禁用 sub-agent 入口，避免子 agent 获得未受控工具集合。

### 3.6 print/single-run mode

可行性：中等。

该模式的价值是把一次 CLI 调用约束为一次 turn，进程退出后释放本次 in-memory session 状态。它适合迁移期验证“每 turn 独立执行”的行为，也适合作为自动化入口。

该模式不能单独满足云端 Runtime 目标。原因如下：

- 独立进程退出只能释放本地进程内对象，不能保证状态已写入云端。
- 如果仍使用本地 rollout 和 SQLite state_db，下一次请求命中其他 App-Server 节点仍无法等价 resume。
- 如果执行环境可写，进程退出后远程 workspace 或本地 workspace 仍保留副作用。
- 如果在长驻 App-Server 内实现为一个 flag，而不是独立进程，则必须显式释放 request-scoped runtime。

当前代码兼容性问题是 `-p` 已被 `--profile` 使用。若希望对齐 Claude Code 的 `-p` 语义，需要选择以下之一：

1. 保留 `-p` 为 profile，新增长参数或子命令表达 print/single-run mode。
2. 在新主命令或云端 CLI wrapper 中使用 `-p`，不改变现有 `codex` 参数。
3. 迁移 `--profile` 短参数，提供兼容期和冲突提示。

## 4. 推荐落地顺序

### 阶段一：边界收敛

- 保持当前本地执行模型不变。
- 为需求新增架构文档和测试计划。
- 明确 `ThreadStore` 是云端状态迁移主边界。
- 标记 rollout path API 为 local compatibility。
- 定义单次请求语义：请求进入时 load/resume，请求结束时 persist/release。
- 梳理所有可写执行面：shell、process、filesystem、patch、MCP、hooks、skill script、sub-agent。
- 明确 print/single-run mode 是独立进程模式还是长驻 App-Server 内部 request-scoped mode。

### 阶段一点五：严格只读 Runtime 原型

- 定义云端 Runtime capability profile。
- 禁用或重路由 app-server 可写 fs/process/shell 接口。
- 将 `command/exec` 强制路由到只读 exec-server。
- 禁止 App-Server 主机 fallback。
- 将 `apply_patch` 替换为 PatchArtifact 输出。
- 禁用 sub-agent 入口。
- 增加写入尝试失败的集成测试。

### 阶段二：云端 ThreadStore 原型

- 新增 store 实现，不改 core session 语义。
- 支持新线程、append item、flush、read、list。
- app-server 通过配置选择云端 store。
- 不支持运行中跨实例接管。
- 支持 writer lease、fencing token 和 append 幂等键。
- 增加任意节点基于 thread id 读取历史并恢复下一 turn 的集成测试。

### 阶段二点五：云端 state_db

- 将 SQLite `state_db` 当前职责梳理为云端状态接口。
- 覆盖 thread metadata、goal、memory mode、backfill state、agent edge metadata 等现有状态读写。
- app-server 和 core 不再直接依赖 SQLite 具体类型。
- 本地 SQLite 仅保留为 LocalThreadStore 或单机兼容实现。

### 阶段三：Skill NFS 加载原型

- 新增 NFS root 配置。
- 先使用挂载路径直接扫描。
- 增加 Skill fingerprint 到加载结果。
- `skills/list` 返回版本信息。

### 阶段四：本地 Skill 缓存

- 新增本地缓存目录。
- 实现 NFS 到本地缓存物化。
- 实现内容哈希、TTL、原子替换、容量清理。
- 将 loader 改为读取缓存路径。

### 阶段五：请求级一致性

- turn 启动时绑定 Skill 指纹。
- 输出事件记录 Skill 指纹。
- Skill 更新只影响新请求。
- 运行中请求不被缓存刷新影响。

### 阶段六：单次请求化

- 将 `turn/start` 或新单次请求 API 改为内部执行 resume。
- 请求完成后释放不再需要的 `CodexThread`。
- 证明连续两次请求命中不同实例时，第二次请求可从云端状态恢复上下文。
- 保留运行中 owner 实例约束，不做运行中接管。
- 运行中 SSE 断线触发当前 turn 取消或中断，不做重新 attach。
- 如果采用 print/single-run mode，验证每次调用结束后进程内对象释放；如果采用长驻 App-Server，验证 release 逻辑。

## 5. 风险等级

| 领域 | 风险 | 说明 |
| --- | --- | --- |
| 云端 ThreadStore | 中 | 有 trait 边界，但本地 rollout path 仍外泄。 |
| Thread writer lease | 中高 | 当前 append 参数缺少 lease/fencing/idempotency，需要扩展写入协议或调用上下文。 |
| 单次请求化 | 中高 | 当前 turn/start 假设线程已加载，需要新增按线程标识恢复并执行的路径。 |
| 实例绑定运行模型 | 中 | 运行期模型可复用，但请求结束后的释放和下次重建需要明确。 |
| 已完成请求查询云端化 | 中低 | 主要可通过 store 实现推进。 |
| state_db 云端化 | 中高 | 当前 SQLite 类型被多处直接引用，需要抽象化并迁移调用方。 |
| 严格只读 Runtime | 高 | app-server、exec-server、filesystem、process、MCP、hooks、skill script 和 patch 均可能形成写入路径，需要统一 capability profile。 |
| 只读 exec-server | 中高 | exec-server 可作为边界复用，但必须禁止 workspace 写入和 App-Server fallback。 |
| PatchArtifact 替代 `apply_patch` | 中高 | 默认 agent 指令和现有编辑路径假设直接修改文件，需要替换为 artifact 输出和后续外部应用流程。 |
| print/single-run mode | 中 | 独立进程可释放进程内状态，但 CLI `-p` 存在参数冲突，且无法替代云端状态和只读执行边界。 |
| SSE 断线取消 | 中 | 协议、transport 和 turn interrupt 需要形成确定终态；不支持重新 attach 会改变客户端体验。 |
| sub-agent 禁用 | 中 | 需要识别所有 sub-agent 注册入口和插件入口，避免能力绕过。 |
| Skill NFS 加载 | 中 | loader 集中，但 root 与 cache 语义需扩展。 |
| Skill 缓存失效 | 中 | 当前只有全量进程内 cache clear，需要版本化缓存。 |
| CLI 兼容 | 中 | app-server 已存在，但 exec/TUI 语义需要分层迁移。 |

## 6. 总体判断

当前代码支持渐进式改造，但不支持一次性完成完全无状态服务。

可优先落地的能力：

- 严格只读 Runtime profile 原型。
- command/exec 只读 exec-server 路由。
- PatchArtifact 替代直接 workspace 修改。
- 云端化已完成线程、事件和元数据读取。
- 新增云端 `ThreadStore`。
- Thread writer lease 和 fencing。
- 任意节点基于线程标识 resume 下一次 turn。
- NFS Skill root 扫描。
- Skill 加载结果版本化。
- Skill 本地缓存和 TTL 失效。
- print/single-run mode 作为迁移期自动化入口。

不纳入当前目标的能力：

- 运行中任务跨实例接管。
- 运行中 SSE 断线重新 attach。
- 审批、租户、外围 Session 管理和 Web 通知编排。
- command exec、PTY、MCP 会话跨实例恢复。
- sub-agent 或多 agent 编排。

需要纳入当前目标但改造成本较高的能力：

- 将 SQLite state_db 完整替换为云端状态服务。
- 关闭所有执行环境写入绕过路径。
- 将直接 patch 写入改为 PatchArtifact。

建议首个工程里程碑定义为：

> 服务实例无本地业务状态依赖；每次用户输入作为单次请求执行；运行中任务允许实例绑定；运行中 SSE 断线取消当前 turn；后续请求允许命中任意实例并基于云端状态 resume；执行环境严格只读；文件修改以 PatchArtifact 输出；已完成状态、历史、事件、state_db 元数据、Skill 权威源和 Skill 版本记录迁移到共享存储。

该里程碑与当前代码结构基本匹配，但风险集中在三个位置：本地 state_db 迁移、执行环境写入面收敛、以及直接 `apply_patch` 到 PatchArtifact 的语义替换。
