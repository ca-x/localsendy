# Localsendy

[![CI](https://github.com/ca-x/localsendy/actions/workflows/ci.yml/badge.svg)](https://github.com/ca-x/localsendy/actions/workflows/ci.yml)
[![Container](https://github.com/ca-x/localsendy/actions/workflows/docker.yml/badge.svg)](https://github.com/ca-x/localsendy/actions/workflows/docker.yml)

Localsendy 是一个面向 Docker 部署的 LocalSend 节点，提供响应式 Web 界面。Rust 服务负责 LocalSend v2 设备发现、加密接收、接收审批、文件发送与存储，浏览器只承担局域网内的控制界面。

[English](README.md)

## 已实现

- 基于 LocalSend 官方 Rust core 的 LocalSend v2 HTTPS 接收端与发送端
- 自动监听多网卡的 IPv4/IPv6 UDP 多播发现，并支持高级接口过滤与 Web 端手动扫描
- 有线与 Wi-Fi 连接同一网络时自动优先有线，断开后运行时切换到 Wi-Fi
- 向一个或多个 LocalSend 设备发送多个文件或剪贴板文本
- 手动接受/拒绝，或通过环境变量在启动时启用自动接受
- 实时显示发送与接收字节进度，并持久化发送和接收历史
- 参考官方 LocalSend 的“发送 / 接收 / 设置”信息架构
- 英文、简体中文、繁体中文基础多语言能力
- 持久化的官方风格多语言随机设备名，支持名称前缀、设备类型和型号配置
- 浅色、深色、跟随系统主题，支持减少动态效果与键盘操作
- 非 root 多阶段 Docker 镜像与 GHCR 发布流程

## 快速开始

LocalSend 依赖 UDP 多播发现，因此 Linux 推荐使用 host 网络：

```bash
docker compose up -d
```

访问 `http://<服务器IP>:52222`，收到的文件保存在 `./data/downloads`。

首次启动会在 `/data/device-identity.json` 生成并持久化官方风格随机身份。如需使用固定名称：

```bash
LOCALSENDY_ALIAS="家庭 NAS" docker compose up -d
```

> Docker Desktop 对 host 网络的支持会随平台和版本变化。Linux 上使用 `network_mode: host` 最可靠。

LocalSend 多播发现只能覆盖同一广播域。跨路由或跨网段时，请在“发送”页面使用手动 IP 探测，或在网络中配置 multicast relay。

Linux 使用 host 网络时，Localsendy 会自动监听宿主机上的全部可用接口，正常使用无需选择。运行中新增的有线、Wi-Fi、桥接、VPN 与隧道接口会被自动发现；高级设置仍可限制指定接口并添加易读备注。自动发现同时支持 LocalSend 的 IPv4 组 `224.0.0.167` 与 IPv6 组 `ff12::fd3a:e420`，网络能够承载 IPv6 多播时也适用于 `fc00::/7` ULA 网络。

当有线和 Wi-Fi 拥有相同的非链路本地网络前缀时，自动模式只通过有线接口广播；有线断开后，五秒一次的接口刷新会启用 Wi-Fi。隧道、Docker bridge 和虚拟接口即使前缀重叠也保持独立。

网络偏好和接口备注会保存到 `/data/network-settings.json`。仅在没有持久配置时，才使用 `LOCALSENDY_NETWORK_INTERFACES` 作为初始回退值。

## 配置

| 环境变量 | 默认值 | 用途 |
| --- | --- | --- |
| `LOCALSENDY_BIND` | `0.0.0.0:52222` | Web UI 与控制 API 监听地址 |
| `LOCALSENDY_ALIAS` | 未设置 | 固定设备名，会覆盖随机名称和前缀 |
| `LOCALSENDY_ALIAS_PREFIX` | 未设置 | 添加到多语言随机名称前的前缀 |
| `LOCALSENDY_ALIAS_LOCALE` | `auto` | 随机名称语言：`auto`、`en`、`zh-CN`、`zh-TW`；`auto` 跟随 `LC_ALL`/`LANG` |
| `LOCALSENDY_DEVICE_TYPE` | `server` | 标准 LocalSend 类型：`mobile`、`desktop`、`web`、`headless`、`server` |
| `LOCALSENDY_DEVICE_MODEL` | 自动检测系统 | 展示给对端的任意型号，例如 `Linux`、`Windows` 或产品名称 |
| `LOCALSENDY_PORT` | `53317` | LocalSend HTTPS 与发现端口 |
| `LOCALSENDY_DATA_DIR` | `/data` | 持久化数据根目录 |
| `LOCALSENDY_DOWNLOAD_DIR` | `/data/downloads` | 接收文件保存目录，会覆盖数据根目录下的默认路径 |
| `LOCALSENDY_TEMP_DIR` | `/data/tmp` | 运行时临时数据目录 |
| `LOCALSENDY_AUTO_ACCEPT` | `false` | 自动接受传入请求；在服务启动时读取 |
| `LOCALSENDY_DISCOVERY_INTERVAL_SECONDS` | `30` | 局域网广播间隔，最小 5 秒 |
| `LOCALSENDY_NETWORK_INTERFACES` | `all` | 初始回退值：`all`、`*` 或接口列表；界面持久化配置优先 |
| `LOCALSENDY_MAX_UPLOAD_BYTES` | `10737418240` | 单次浏览器发送请求的总大小上限 |
| `RUST_LOG` | `localsendy=info,tower_http=info` | Rust 日志过滤器 |

别名、设备类型、设备型号和端口属于启动身份。修改环境变量并重启容器后生效，与官方 LocalSend 的服务重启行为一致。

## 当前协议边界

Localsendy 只展示当前服务能够真实执行的设置。自动接受通过环境变量配置，保存目录可在 Web 端选择；PIN 校验、收藏设备信任规则、链接分享和自定义多播组目前尚未实现，因此不会提供看似可用但实际无效的开关。

## 本地开发与检查

需要 Rust 1.97+、Node.js 24+、npm 12+。

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
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
docker build -t localsendy:dev .
```

更多信息见 [架构文档](docs/architecture.md) 与已持久化的 [UI 设计系统](design-system/localsendy/MASTER.md)。

## 说明

产品流程与互操作目标参考官方 [LocalSend](https://github.com/localsend/localsend)。Localsendy 是独立项目，与 LocalSend 官方没有从属或背书关系。

协议实现来自 LocalSend 使用 MIT 许可证的官方 Rust core，上游版本记录在 [`third_party/localsend-core/UPSTREAM.md`](third_party/localsend-core/UPSTREAM.md)。本项目使用 MIT 许可证。
