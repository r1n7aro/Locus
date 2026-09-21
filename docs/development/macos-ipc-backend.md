# macOS Unity 基础通信后端

本实现仅由 `target_os = "macos"` 编译，Windows named-pipe / MMF / hook / overlay 模块保持原样。C ABI 保留 `locus_init`、生命周期、心跳、poll、complete、event 等现有入口；hook 与 overlay 在 Mac 继续返回不支持，broker 同时拒绝 `hot_reload_*` 和 `hot_patch_*` 请求。

## Endpoint 与身份

- endpoint：`/tmp/locus-<euid>/<project-key>[-namespace-key].sock`。`project-key` 为规范化绝对项目路径 UTF-8 的 SHA-256 前 16 字节小写十六进制；保留路径大小写。可选 `namespace-key` 为 `LOCUS_UNITY_TEST_PIPE_NAMESPACE` 的 SHA-256 前 8 字节小写十六进制。
- desktop 将完整 endpoint 写入现有 `Library/Locus/NativeBridge.enabled`；C# Mac 分支读取原值，native 根据项目路径再次验证。Mac 无 marker 时不回退到 Windows 命名规则。
- 当前用户目录要求自身是目录、当前 UID 所有、权限 0700；不接受该目录是符号链接。socket 要求当前 UID 所有、0600，并校验路径长度小于 Darwin 的 104 字节上限。
- 每 endpoint 用 0600、`O_NOFOLLOW` 的锁文件和 `flock(LOCK_EX | LOCK_NB)` 确保单 broker。锁文件 inode 保留，避免 unlink 锁造成双持有。未持锁不清理 socket；持锁后仍拒绝已有 live listener。
- 连接双方读取内核 peer UID/PID。desktop 验证 peer PID 等于状态文件 PID，并核验 `proc_pidinfo(PROC_PIDTBSDINFO)` 返回的 UID、PID、启动秒/微秒；不能将未知身份当成成功。

## 请求与状态

命令协议仍为 newline JSON envelope，保留 `id` / `reply_to`、`type`、`message`、`ok`、`error`、`processId`、`processPath`。ACK 使用 `locus-request-accepted`。Mac 客户端请求 ID 增加 desktop 进程和会话标识，避免跨 desktop 重启的旧完成结果对应到新请求。

Mac broker 保留有界 queue、inflight、deadline、reload 中断和 execution 重连语义。进入 reload/quit 或观察到新 generation 时终止旧请求，旧 generation 心跳与完成不能恢复旧工作。断开时丢弃普通 queued/inflight 请求，保留可按 managed execution ID 重新附着的执行请求。取消仍使用 managed 的 `cancel_execute_code` 请求；本地取消不会假装已撤回 broker 已接受的工作。

独立状态文件为同 stem 的 `.state.json`，payload 沿用现有状态和事件 cursor 语义，新增 `stateVersion=1`、`processStartSecs`、`processStartMicros`。broker 后台每 250ms 发布原子快照，即使 managed 主线程暂停仍可读取 native 存活与最后 managed 心跳。文件使用 0600、新建临时文件后 rename；读取限制为 128KiB，并拒绝符号链接、多 hardlink、宽权限、不同项目/endpoint、过期超过 3 秒或未来超过 1 秒的快照。

命令读取使用有界 frame，避免无换行或超大消息无限增长。响应 writer 队列溢出或写超时会断开连接，使客户端 pending 请求显式失败。正常退出清理本 broker 的 socket 和状态文件；异常退出遗留由下一次持有锁的 broker 检查后处理，陈旧快照不能继续证明进程存活。

## 验证

参考 [Windows 后端保护证据](windows-backend-preservation.md)。Windows 本机已完成两种 Darwin 架构的 native 与 Mac transport 类型检查；Mac socket/Unity 实机运行和各生命周期场景仍需在 macOS 验收。
