# 部署

弱机器默认 `low`（约 75MB）。安装脚本只需要 **curl** 和 **tar**：按平台从 GitHub Release 拉预编译二进制，不装 git、也不编译。

```bash
curl -fsSL https://raw.githubusercontent.com/toolazytoname/plutus-rustus/main/install.sh | bash -s --
```

要更高一档、并立刻拉地址库再跑：

```bash
PLUTUS_BARK_KEY=你的Bark密钥 \
curl -fsSL https://raw.githubusercontent.com/toolazytoname/plutus-rustus/main/install.sh \
  | bash -s -- --profile=balanced --fetch-db --start
```

钉某个版本（生产建议）：

```bash
curl -fsSL https://raw.githubusercontent.com/toolazytoname/plutus-rustus/main/install.sh \
  | bash -s -- --version=v0.2.2 --profile=low
```

`main` 上每次 CI 通过会刷新 `nightly` prerelease：`--nightly` 或 `--version=nightly`。没有正式 tag 时，默认 `latest` 会回落到 nightly。

| 参数 | 默认 | 作用 |
|---|---|---|
| `--profile=low\|balanced\|full` | `low` | 线程 / CPU / 未压缩地址 |
| `--dir PATH` | `~/plutus-rustus`（root 则 `/opt/plutus-rustus`） | 安装目录；进程名是 `goldpan` |
| `--version` | `latest` | Release tag；`latest` 没有则用 `nightly` |
| `--fetch-db` | 关 | 现在就下载全量地址库（约 1.4GB） |
| `--start` | 关 | 装完直接启动 |
| `--from-source` | 关 | 开发者：clone + `cargo` 本机编译（需要 git 和 Rust） |

Bark 密钥走环境变量，不要写进命令行参数（`ps` 能看见 argv）。脚本会写进 `.env`，不会打印出来。

Linux 二进制是 **musl 静态链接**，旧版 glibc 的 Debian/Ubuntu VPS 也能跑。macOS 提供 aarch64 和 x86_64。

已经装过的机器：`~/plutus-rustus/shell/plutus upgrade`（再拉一次 Release，保留 `config.toml` / `.env` / `data/`）。

## 装好之后

```bash
~/plutus-rustus/shell/plutus doctor
~/plutus-rustus/shell/plutus status
~/plutus-rustus/shell/plutus logs
~/plutus-rustus/shell/plutus stop
~/plutus-rustus/shell/plutus upgrade
```

有 sudo 时会装 `goldpan.service`（崩溃自动拉起、开机自启、`CPUQuota` 随 profile、`MemoryMax=256M`、日志在 journal）。没有 sudo 就用仓库里的 shell 看门狗。进程和 tarball 叫 `goldpan`，安装目录仍可以是 `~/plutus-rustus`。

## 档位

| 档位 | 适用 | 地址表大约占内存 |
|---|---|---|
| `low` | 1 核小 VPS | ~75MB |
| `balanced` | 普通云主机 | ~85MB |
| `full` | 空闲工作站 | ~100MB |

进程 RSS 会再高一点。更新地址库时先丢掉表再下载，峰值大约一两百 MB。**256MB 内存能跑 `low`；512MB 起更稳；1GB 很宽裕。** 不是苛刻的机器要求。

## 地址库

默认 `auto_update=true`：快照超过 30 小时，引擎丢掉 Bloom、下载、再继续。不要再开 `goldpan-update.timer`。

第一次没带 `--fetch-db` 的话，启动前跑一次：

```bash
~/plutus-rustus/shell/plutus update-db
~/plutus-rustus/shell/plutus start
```

地址库仍然从 Loyce 拉 gzip（约 1.4GB），不进 GitHub Release（超过合理附件大小）。

## Bark

iOS 装 [Bark](https://github.com/Finb/Bark)，设备密钥放到 `PLUTUS_BARK_KEY`。推送不含私钥：启动、每天「还活着」、正在更新、命中（只有地址）、停止。

## CI / CD

`.github/workflows/ci.yml`：PR 只跑 format / clippy / test。push 到 `main` 或打 `v*` tag 时，verify 通过后按四个 target 出包并上传 Release：

- `goldpan-linux-x86_64.tar.gz`（musl）
- `goldpan-linux-aarch64.tar.gz`（musl）
- `goldpan-macos-aarch64.tar.gz`
- `goldpan-macos-x86_64.tar.gz`

每个包带 `.sha256`。安装脚本校验后再解压。正式发版：`git tag v0.2.2 && git push origin v0.2.2`。
