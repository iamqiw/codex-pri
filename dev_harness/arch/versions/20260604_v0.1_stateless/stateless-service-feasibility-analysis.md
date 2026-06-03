# Codex CLI 无状态服务可行性分析

## 1. 分析范围

本分析基于当前代码结构，评估以下目标的可行性：

- 将 Codex CLI/app-server 的业务状态迁移到云端持久化。
- 降低服务实例对进程内长期状态的依赖。
- 将长链接对话改造为单次请求对话。
- 支持几十到上百个云端 Codex 实例中任意节点基于线程标识 resume 后续请求。
- 将 Skill 权威源改为 NFS，并在服务实例本地缓存。
- 为 Skill 更新提供缓存失效机制。

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

结论：查询类 API 可较快云端化；运行中请求、活动 turn、审批等待、取消、流式订阅继续保持实例绑定。当前目标不包含运行中任务跨实例接管、审批脱离连接、PTY/MCP 会话恢复。单次请求模型要求 turn 结束后释放进程内线程对象，并保证下一次请求可由任意节点从云端状态重建上下文。

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

建议分阶段：

1. 新增云端 `ThreadStore` 实现，仅覆盖 create/resume/append/flush/read/list/archive。
2. 将 app-server 查询路径切到 store-first，避免扫描本地 rollout。
3. 收敛 `read_thread_by_rollout_path` 为 local-only 兼容路径。
4. 将 state_db 访问抽象为 trait，再实现云端 DB 版本，并覆盖 goal、thread metadata、memory mode、backfill state 等现有 SQLite 职责。

### 3.2 单次请求与实例绑定运行模型

可行性：中等偏高，但需要收敛当前 long-lived thread 假设。

当前目标定义为“每次用户输入是一次独立请求；请求运行期间可以绑定实例；请求完成后后续请求必须能由任意实例 resume”。该目标保留当前 `ThreadManager`、`CodexThread`、listener、审批和 PTY/MCP 的运行期模型，但要求请求边界结束后不能依赖这些进程内对象。

明确非目标：

- 不做运行中任务跨实例接管。
- 不做审批脱离连接或脱离运行实例。
- 不做 command exec、PTY、shell session、MCP 会话跨实例恢复。

建议首版约束：

- 查询、历史、事件读取、归档、元数据更新无状态化。
- 每次 turn start 前从云端状态加载线程历史、配置快照和 state_db 元数据。
- 每次 turn 完成或中断后关闭或可回收进程内 `CodexThread`。
- 运行中请求路由到 owner 实例；后续新请求不得要求命中同一实例。
- 实例异常时，运行中请求进入 `interrupted` 或 `failed`，不做跨实例继续执行。
- 审批最终结果写入云端状态，但审批处理可继续依赖活动连接。

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

## 4. 推荐落地顺序

### 阶段一：边界收敛

- 保持当前本地执行模型不变。
- 为需求新增架构文档和测试计划。
- 明确 `ThreadStore` 是云端状态迁移主边界。
- 标记 rollout path API 为 local compatibility。
- 定义单次请求语义：请求进入时 load/resume，请求结束时 persist/release。

### 阶段二：云端 ThreadStore 原型

- 新增 store 实现，不改 core session 语义。
- 支持新线程、append item、flush、read、list。
- app-server 通过配置选择云端 store。
- 不支持运行中跨实例接管。
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

## 5. 风险等级

| 领域 | 风险 | 说明 |
| --- | --- | --- |
| 云端 ThreadStore | 中 | 有 trait 边界，但本地 rollout path 仍外泄。 |
| 单次请求化 | 中高 | 当前 turn/start 假设线程已加载，需要新增按线程标识恢复并执行的路径。 |
| 实例绑定运行模型 | 中 | 运行期模型可复用，但请求结束后的释放和下次重建需要明确。 |
| 已完成请求查询云端化 | 中低 | 主要可通过 store 实现推进。 |
| state_db 云端化 | 中高 | 当前 SQLite 类型被多处直接引用，需要抽象化并迁移调用方。 |
| Skill NFS 加载 | 中 | loader 集中，但 root 与 cache 语义需扩展。 |
| Skill 缓存失效 | 中 | 当前只有全量进程内 cache clear，需要版本化缓存。 |
| CLI 兼容 | 中 | app-server 已存在，但 exec/TUI 语义需要分层迁移。 |

## 6. 总体判断

当前代码支持渐进式改造，但不支持一次性完成完全无状态服务。

可优先落地的能力：

- 云端化已完成线程、事件和元数据读取。
- 新增云端 `ThreadStore`。
- 任意节点基于线程标识 resume 下一次 turn。
- NFS Skill root 扫描。
- Skill 加载结果版本化。
- Skill 本地缓存和 TTL 失效。

不纳入当前目标的能力：

- 运行中任务跨实例接管。
- 审批资源完全脱离连接和进程内 listener。
- command exec、PTY、MCP 会话跨实例恢复。

需要纳入当前目标但改造成本较高的能力：

- 将 SQLite state_db 完整替换为云端状态服务。

建议首个工程里程碑定义为：

> 服务实例无本地业务状态依赖；每次用户输入作为单次请求执行；运行中任务允许实例绑定；后续请求允许命中任意实例并基于云端状态 resume；已完成状态、历史、事件、state_db 元数据、Skill 权威源和 Skill 版本记录迁移到共享存储。

该里程碑与当前代码结构匹配，风险可控。
