# Localsendy

[![CI](https://github.com/ca-x/localsendy/actions/workflows/ci.yml/badge.svg)](https://github.com/ca-x/localsendy/actions/workflows/ci.yml)
[![Container](https://github.com/ca-x/localsendy/actions/workflows/docker.yml/badge.svg)](https://github.com/ca-x/localsendy/actions/workflows/docker.yml)

Localsendy 是一个面向 Docker 部署的 LocalSend 节点，提供响应式 Web 界面。Rust 服务负责 LocalSend v2 设备发现、加密接收、接收审批、文件发送与存储，浏览器只承担局域网内的控制界面。

[English](README.md)

## 已实现

- 基于 [`localsend-rs`](https://github.com/CrossCopy/localsend-rs) 的 LocalSend v2 HTTPS 接收端
- 自动监听多网卡的 IPv4/IPv6 UDP 多播发现，并支持高级接口过滤与 Web 端手动扫描
- 向已发现的 LocalSend 设备发送多个文件
- 手动接受/拒绝，或显式开启自动接受
- 接收历史和发送任务状态
- 参考官方 LocalSend 的“发送 / 接收 / 设置”信息架构
- 英文、简体中文、繁体中文基础多语言能力
- 浅色、深色、跟随系统主题，支持减少动态效果与键盘操作
- 非 root 多阶段 Docker 镜像与 GHCR 发布流程

## 快速开始

LocalSend 依赖 UDP 多播发现，因此 Linux 推荐使用 host 网络：

```bash
docker compose up -d
```

访问 `http://<服务器IP>:8080`，收到的文件保存在 `./data/downloads`。

修改设备名称：

```bash
LOCALSENDY_ALIAS="家庭 NAS" docker compose up -d
```

> Docker Desktop 对 host 网络的支持会随平台和版本变化。Linux 上使用 `network_mode: host` 最可靠。

LocalSend 多播发现只能覆盖同一广播域。跨路由或跨网段时，请在“发送”页面使用手动 IP 探测，或在网络中配置 multicast relay。

Linux 使用 host 网络时，Localsendy 会自动监听宿主机上的全部可用接口，正常使用无需选择。运行中新增的有线、Wi-Fi、桥接、VPN 与隧道接口会被自动发现；高级设置仍可限制指定接口并添加易读备注。自动发现同时支持 LocalSend 的 IPv4 组 `224.0.0.167` 与 IPv6 组 `ff12::fd3a:e420`，网络能够承载 IPv6 多播时也适用于 `fc00::/7` ULA 网络。

网络偏好和接口备注会保存到 `/data/network-settings.json`。仅在没有持久配置时，才使用 `LOCALSENDY_NETWORK_INTERFACES` 作为初始回退值。

## 配置

| 环境变量 | 默认值 | 用途 |
| --- | --- | --- |
| `LOCALSENDY_BIND` | `0.0.0.0:8080` | Web UI 与控制 API 监听地址 |
| `LOCALSENDY_ALIAS` | `Localsendy` | 展示给其它 LocalSend 设备的名称 |
| `LOCALSENDY_PORT` | `53317` | LocalSend HTTPS 与发现端口 |
| `LOCALSENDY_DATA_DIR` | `/data` | 持久化数据根目录 |
| `LOCALSENDY_DOWNLOAD_DIR` | `/data/downloads` | 接收文件保存目录，会覆盖数据根目录下的默认路径 |
| `LOCALSENDY_AUTO_ACCEPT` | `false` | 是否无需浏览器批准自动接收 |
| `LOCALSENDY_DISCOVERY_INTERVAL_SECONDS` | `30` | 局域网广播间隔，最小 5 秒 |
| `LOCALSENDY_NETWORK_INTERFACES` | `all` | 初始回退值：`all`、`*` 或接口列表；界面持久化配置优先 |
| `LOCALSENDY_MAX_UPLOAD_BYTES` | `10737418240` | 单次浏览器发送请求的总大小上限 |
| `RUST_LOG` | `localsendy=info,tower_http=info` | Rust 日志过滤器 |

## 本地开发与检查

需要 Rust 1.94+、Node.js 24+、npm 12+。

```bash
npm --prefix web ci
npm --prefix web run build
cargo run
```

完整检查：

```bash
npm --prefix web run typecheck
npm --prefix web run test:ci
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
docker build -t localsendy:dev .
```

更多信息见 [架构文档](docs/architecture.md) 与已持久化的 [UI 设计系统](design-system/localsendy/MASTER.md)。

## 说明

产品流程与互操作目标参考官方 [LocalSend](https://github.com/localsend/localsend)。Localsendy 是独立项目，与 LocalSend 官方没有从属或背书关系。

协议实现使用 MIT 许可证的 [`localsend-rs`](https://github.com/CrossCopy/localsend-rs)。本项目使用 MIT 许可证。
