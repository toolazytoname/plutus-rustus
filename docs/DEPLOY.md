# 部署与运维

弱机器也能跑。默认查找是 **mmap**：Bloom 过滤器加 64K 分桶索引留在内存（约 75–100MB），20 字节 hash160 表留在磁盘上按桶 `pread`。`sorted`（~900MB）和 `hash`（~1.3GB）只留给对照调试。更新地址库时引擎会先丢掉内存里的过滤器再下载，并且用外排只保留约 20MB 的排序块，所以不要一边跑引擎一边另开进程更新。

## 1. 本机第一次安装

```bash
git clone --recursive <your-fork-url> plutus-rustus
cd plutus-rustus
bash shell/install_start.sh
```

然后编辑两个被 git 忽略的文件：

1. `.env`：填 `PLUTUS_BARK_KEY`（Bark App 里的设备密钥）和 `PLUTUS_NODE_NAME`（多机时用来区分）
2. `config.toml`：弱 VPS 保持 `profile = "low"`

验证推送和配置：

```bash
./target/release/plutus-rustus doctor
./target/release/plutus-rustus notify-test
bash shell/start.sh
bash shell/status.sh
bash shell/logs.sh
```

停：`bash shell/stop.sh`

## 2. 弱机器怎么配

| 档位 | 适用 | 线程 | CPU | 查找 | 未压缩地址 | 大约内存 |
|---|---|---|---|---|---|---|
| `profile = "low"` | 1 核 / 1–2GB VPS | 1 | 40% | mmap | 关 | ~75MB |
| `profile = "balanced"` | 普通云主机 | 全核 | 70% | mmap | 开 | ~85MB |
| `profile = "full"` | 空闲工作站 | 全核 | 100% | mmap | 开 | ~100MB |

另外两层限制，可以叠加：

- 程序内：`cpu_percent = 40` 或环境变量 `PLUTUS_CPU_PERCENT=40`（算一阵、睡一阵）
- systemd：`CPUQuota=40%`（cgroup 硬限制，`shell/install-systemd.sh` 默认就写这个）

**1GB 内存**：用 `low`，不要开 systemd 更新 timer。  
**内存小于 512MB**：不建议跑（操作系统 + 下载缓冲会挤掉 Bloom）。

程序内限速是“礼让”，不是精确的 40% 核；要硬限制用 systemd `CPUQuota`。

## 3. 地址库要不要每天拉

要。有余额的地址集合每天都在变，旧库等于对着过期目标搜。

默认 `auto_update = true`：快照超过 `max_snapshot_age_hours = 30` 后，引擎会：

1. 停 worker
2. 丢掉内存里的集合（避免和下载叠成两份内存）
3. 拉 Loyce 全量压缩包，写成 `data/addresses.h160`
4. 重新加载，继续跑
5. Bark 会推「正在更新地址库」；失败会推「更新失败」，然后用磁盘上的旧快照接着跑

不要同时再开 `plutus-update.timer`，否则下载和引擎会抢内存。

手动强制更新：

```bash
bash shell/update.sh
```

## 4. Bark

比 ServerChan 更适合个人手机：延迟低、分组干净、可以自建。

1. iOS 安装 [Bark](https://github.com/Finb/Bark)
2. 复制设备密钥到 `.env` 的 `PLUTUS_BARK_KEY`
3. 自建服务器时设 `PLUTUS_BARK_SERVER=https://your-bark-host`

推送内容（不含私钥）：

- 启动：`Plutus 已启动`
- 每天一次：`Plutus 还活着`（速率、运行时间、快照年龄）
- 快照过期：`Plutus 正在更新地址库`
- 命中：只推地址
- 进程退出：`Plutus 已停止`

一天没收到「还活着」，就是挂了或网络断了。

## 5. systemd（可选）

`auto_update = true` 时只装主服务，不要 enable 更新 timer。

```bash
sudo bash shell/install-systemd.sh
sudo systemctl start plutus
```

`CPUQuota` 默认 40%，可用 `PLUTUS_CPU_QUOTA=25% sudo bash shell/install-systemd.sh` 覆盖。

## 6. Git 历史和密钥

查过当前仓库的 git 历史：

- **真实的 ServerChan key、dufs 密码、内网/公网账号没有进过 git**。它们只出现在本机未跟踪的旧脚本里，那些脚本已经改成读环境变量。
- `master` 上曾经用环境变量名 `SENDKEY`，并且有过 `info!("SENDKEY: {}", key)`，会把**运行时**的 key 打进日志。当前 `codex/revive-plutus` 分支没有这段逻辑。
- 历史上的 `plutus.txt` 是上游 README 里的示例私钥格式，不是你的真实钱包。

因此**没有做 history rewrite**（pickle 数据库在历史里很大，rewrite 还要 force-push 所有分支）。该轮换的是已经在旧脚本/聊天/日志里出现过的 Bark/ServerChan/主机密码，不是 git blob。

如果 Bark key 曾经写进过某处日志，去 Bark 里重置设备密钥。

## 7. 常用命令

```bash
./target/release/plutus-rustus doctor
./target/release/plutus-rustus notify-test
./target/release/plutus-rustus data inspect
./target/release/plutus-rustus data update
bash shell/start.sh
bash shell/status.sh
bash shell/logs.sh
bash shell/stop.sh
```
