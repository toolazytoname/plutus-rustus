# 部署

一条分支、一个安装命令、systemd 看管进程。地址库由引擎自己更新，不要再开第二个下载进程。

默认查找是 mmap：Bloom + 分桶索引大约 **75–100MB**，hash160 表在磁盘上。

## 第一次

```bash
git clone --recursive <your-fork-url> plutus-rustus
cd plutus-rustus
./install.sh
```

然后只改两个 git 忽略的文件：

1. `.env`：`PLUTUS_BARK_KEY`（Bark 设备密钥）、`PLUTUS_NODE_NAME`（多机时用来区分）
2. `config.toml`：弱 VPS 保持 `profile = "low"`

```bash
./shell/plutus doctor
./target/release/plutus-rustus notify-test
./shell/plutus start
./shell/plutus status
./shell/plutus logs
```

停：`./shell/plutus stop`  
以后升级代码：`./shell/plutus upgrade`

有 sudo 时 `install.sh` 会装 `plutus.service`（崩溃自动拉起、`CPUQuota=40%`）。没有 sudo 就用仓库里的 shell 看门狗。

## 弱机器

| 档位 | 适用 | 线程 | CPU | 大约内存 |
|---|---|---|---|---|
| `profile = "low"` | 1 核 / 1–2GB VPS | 1 | 40% | ~75MB |
| `profile = "balanced"` | 普通云主机 | 全核 | 70% | ~85MB |
| `profile = "full"` | 空闲工作站 | 全核 | 100% | ~100MB |

程序内 `cpu_percent` 是礼让；硬限制用 systemd `CPUQuota`（`PLUTUS_CPU_QUOTA=25% sudo bash shell/install-systemd.sh`）。

**1GB 内存**：`low` 即可。  
**小于 512MB**：不建议跑。

## 地址库

`auto_update = true`（默认）：快照超过 30 小时，引擎丢掉内存里的 Bloom、下载 Loyce 全量包、写成 `data/addresses.h160`、再继续跑。Bark 会推「正在更新」；失败则用旧快照接着跑。

不要再 enable `plutus-update.timer`，否则会和引擎抢内存。手动强制更新：

```bash
./shell/plutus update-db
```

第一次没有快照时，`start` 之前跑一次 `update-db`（大约下载 1.4GB 压缩包，写盘，内存峰值仍大约一两百 MB）。

## Bark

1. iOS 装 [Bark](https://github.com/Finb/Bark)
2. 设备密钥放到 `.env` 的 `PLUTUS_BARK_KEY`
3. 自建则设 `PLUTUS_BARK_SERVER`

推送不含私钥：启动、每天「还活着」、正在更新、命中（只有地址）、停止。一天没「还活着」就是挂了或网络断了。

## 设计上为什么这样

- **进程**：交给 systemd（或薄薄一层 shell 循环）。不要再叠一层自写 supervisor。
- **地址库**：引擎内部更新，更新前先释放 Bloom，避免双份内存。
- **密钥**：只在 `.env`，脚本和通知都不带私钥。
- **代码升级**：`git pull` + 本地编一次 native binary，不走额外的文件服务器。
