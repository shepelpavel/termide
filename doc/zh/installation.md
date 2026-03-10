# 安装指南

本指南介绍在您的系统上安装 TermIDE 的不同方法。

## 下载预编译二进制文件（推荐）

最简单的入门方式是下载适合您平台的预编译二进制文件。

### 第 1 步：下载

访问 [GitHub Releases](https://github.com/termide/termide/releases) 页面，下载适合您平台的最新版本：

**Linux x86_64**（也适用于 WSL/WSL2）：
```bash
wget https://github.com/termide/termide/releases/latest/download/termide-0.14.3-x86_64-unknown-linux-gnu.tar.gz
```

**Linux ARM64**（树莓派、ARM 服务器）：
```bash
wget https://github.com/termide/termide/releases/latest/download/termide-0.14.3-aarch64-unknown-linux-gnu.tar.gz
```

**macOS Intel (x86_64)**：
```bash
curl -LO https://github.com/termide/termide/releases/latest/download/termide-0.14.3-x86_64-apple-darwin.tar.gz
```

**macOS Apple Silicon (M1/M2/M3)**：
```bash
curl -LO https://github.com/termide/termide/releases/latest/download/termide-0.14.3-aarch64-apple-darwin.tar.gz
```

### 第 2 步：解压

```bash
tar xzf termide-*.tar.gz
```

### 第 3 步：运行

```bash
./termide
```

### 第 4 步：全局安装（可选）

要将 TermIDE 安装到系统中，请将二进制文件移动到 PATH 中的目录：

```bash
# Linux
sudo mv termide /usr/local/bin/

# macOS
sudo mv termide /usr/local/bin/
```

现在您可以在终端的任何位置运行 `termide`。

## 通过包管理器安装

### Debian/Ubuntu (.deb)

```bash
wget https://github.com/termide/termide/releases/latest/download/termide_0.14.3-1_amd64.deb
sudo dpkg -i termide_0.14.3-1_amd64.deb
```

### Fedora/RHEL/CentOS (.rpm)

```bash
wget https://github.com/termide/termide/releases/latest/download/termide-0.14.3-1.x86_64.rpm
sudo rpm -i termide-0.14.3-1.x86_64.rpm
```

### Arch Linux (AUR)

```bash
# 从源码构建
yay -S termide

# 或安装预编译二进制文件
yay -S termide-bin
```

### Homebrew (macOS/Linux)

```bash
brew tap termide/termide
brew install termide
```

### NixOS/Nix (Flakes)

```bash
# 无需安装直接运行
nix run github:termide/termide

# 安装到用户配置文件
nix profile install github:termide/termide
```

## 从源码构建

### 前置条件

- **Rust 1.70+**（stable 工具链）
- **Git**

### 使用 Cargo

```bash
# 克隆仓库
git clone https://github.com/termide/termide.git
cd termide

# 以 release 模式构建
cargo build --release

# 二进制文件位于 target/release/termide
./target/release/termide

# 可选：安装到 ~/.cargo/bin
cargo install --path .
```

### 使用 Nix（配合 Flakes）

```bash
# 克隆仓库
git clone https://github.com/termide/termide.git
cd termide

# 进入开发 shell
nix develop

# 使用 cargo 构建
cargo build --release

# 或直接使用 Nix 构建
nix build
```

## 平台特定说明

### Linux

预编译二进制文件无需额外依赖。

从源码构建时，可能需要安装开发包：
```bash
# Debian/Ubuntu
sudo apt-get install build-essential

# Fedora/RHEL
sudo dnf install gcc
```

### macOS

首次运行时，macOS 可能会因为应用程序未签名而阻止运行。要允许运行：
1. 右键点击 `termide` 并选择"打开"
2. 在安全对话框中点击"打开"

或者，移除隔离属性：
```bash
xattr -d com.apple.quarantine termide
```

### Windows (WSL)

TermIDE 可在 Windows Subsystem for Linux（WSL 和 WSL2）中运行：

1. 如果尚未安装，请先安装 WSL2
2. 在 WSL 中下载 Linux x86_64 二进制文件：
   ```bash
   wget https://github.com/termide/termide/releases/latest/download/termide-0.14.3-x86_64-unknown-linux-gnu.tar.gz
   tar xzf termide-0.14.3-x86_64-unknown-linux-gnu.tar.gz
   ./termide
   ```

## 验证安装

安装完成后，验证是否正常工作：

```bash
termide --version
```

## 下一步

- 阅读[用户界面指南](ui.md)了解应用程序布局
- 了解[文件管理器](file-manager.md)键盘快捷键
- 探索[终端](terminal.md)和[编辑器](editor.md)功能
- 使用[主题](themes.md)自定义您的体验
