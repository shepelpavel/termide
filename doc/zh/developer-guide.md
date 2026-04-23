# 开发者指南

本指南面向希望参与 TermIDE 开发或了解其代码库的开发者。

## 开发环境搭建

### 前置条件

- **Rust 1.70+**（stable 工具链）
- **Git** 版本控制
- **可选：** 启用 flakes 的 Nix，用于可重复构建

### 获取源代码

```bash
git clone https://github.com/termide/termide.git
cd termide
```

### 构建

#### 使用 Cargo（标准方式）

```bash
# 开发构建
cargo build

# 带优化的发布构建
cargo build --release

# 以开发模式运行
cargo run

# 以发布模式运行
cargo run --release
```

#### 使用 Nix（可重复构建）

```bash
# 进入包含所有依赖的开发 shell
nix develop

# 使用 Nix 构建
nix build

# 运行检查
nix flake check
```

### 运行测试

```bash
# 运行所有测试
cargo test

# 运行测试并显示输出
cargo test -- --nocapture

# 运行特定测试
cargo test test_name
```

### 代码质量检查

```bash
# 检查编译错误
cargo check

# 运行 clippy 代码检查器
cargo clippy

# 格式化代码
cargo fmt

# 检查格式但不修改文件
cargo fmt --check
```

## 项目结构

TermIDE 使用 Cargo workspace，采用模块化 crate 架构：

```
termide/
├── src/                       # 二进制入口点
│   ├── main.rs               # 应用初始化、终端设置
│   └── ui.rs                 # 顶层渲染桥接
├── crates/
│   ├── app/                  # 应用核心、事件处理、面板管理
│   ├── app-core/             # 核心应用 trait（LayoutController、PanelProvider）
│   ├── app-event/            # 事件处理逻辑和快捷键处理
│   ├── app-modal/            # 模态对话框处理
│   ├── app-panel/            # 面板管理操作
│   ├── app-session/          # 会话保存/恢复逻辑
│   ├── app-watcher/          # 文件系统监视器集成
│   ├── buffer/               # 文本缓冲区实现（基于 ropey）
│   ├── clipboard/            # 系统剪贴板集成
│   ├── config/               # 配置管理（TOML）
│   ├── core/                 # 核心 Panel trait 和共享类型
│   ├── file-ops/             # 文件操作（复制、移动、删除、上传、下载）
│   ├── git/                  # Git 集成（状态、差异、日志）
│   ├── highlight/            # 语法高亮（tree-sitter，15+ 种语言）
│   ├── i18n/                 # 国际化（15 种语言）
│   ├── keyboard/             # 键盘处理和布局翻译
│   ├── layout/               # 面板布局和手风琴系统
│   ├── logger/               # 日志系统
│   ├── lsp/                  # 语言服务器协议客户端
│   ├── modal/                # 模态对话框实现
│   ├── panel-diagnostics/    # LSP 诊断面板
│   ├── panel-editor/         # 文本编辑器面板
│   ├── panel-file-manager/   # 文件管理器面板
│   ├── panel-git-diff/       # Git 差异查看器面板
│   ├── panel-git-log/        # Git 日志面板
│   ├── panel-git-status/     # Git 状态面板
│   ├── panel-image/          # 图片查看器面板
│   ├── panel-misc/           # 欢迎界面和日志面板
│   ├── panel-operations/     # 后台操作面板
│   ├── panel-terminal/       # 终端模拟器面板（PTY）
│   ├── session/              # 会话持久化
│   ├── state/                # 应用状态（批量、布局、操作、UI）
│   ├── system-monitor/       # CPU/内存/磁盘监控
│   ├── theme/                # 主题系统和 38 款内置主题
│   ├── ui/                   # UI 工具和路径格式化
│   ├── ui-render/            # UI 渲染（菜单、状态栏、面板）
│   ├── vfs/                  # 虚拟文件系统（SFTP、FTP、SMB）
│   └── watcher/              # 文件系统事件监视器
├── doc/                       # 文档
│   ├── en/                   # 英文文档
│   ├── ru/                   # 俄文文档
│   └── zh/                   # 中文文档
└── packaging/                 # 分发打包（deb、rpm、AUR、Homebrew、Nix）
```

## 关键组件

### 1. LayoutManager (`crates/layout/src/`)

管理手风琴面板布局系统：
- 管理水平面板组（`Vec<PanelGroup>`）
- 处理焦点导航（Alt+Left/Right 在组之间切换）
- 智能面板堆叠/拆分（Alt+Backspace）
- 通过 `setup_default_layout()` 实现宽度自适应默认布局

### 2. PanelGroup (`crates/layout/src/panel_group.rs`)

表示面板的垂直堆叠（手风琴式）：
- 一个展开的面板，其他折叠为标题栏
- 维护 `expanded_index`
- `width: Option<u16>` 用于显式宽度控制
- 提供组内导航（Alt+Up/Down）

### 3. Panel Trait (`crates/core/src/lib.rs`)

所有面板实现此 trait：
```rust
pub trait Panel {
    fn render(&mut self, area: Rect, buf: &mut Buffer, is_focused: bool, panel_index: usize, state: &AppState);
    fn handle_key(&mut self, key: KeyEvent) -> Result<()>;
    fn handle_mouse(&mut self, mouse: MouseEvent, panel_area: Rect) -> Result<()>;
    fn title(&self) -> String;
    fn is_welcome_panel(&self) -> bool { false }
    // ... 其他方法
}
```

### 4. 事件处理 (`crates/app/src/app/`)

**流程：**
1. `EventHandler` 轮询终端事件
2. 事件分发到相应的处理器：
   - `key_handler.rs` 处理键盘
   - `mouse_handler.rs` 处理鼠标
   - `modal_handler.rs` 处理模态框
3. 处理器更新 `LayoutManager` 和面板状态
4. 在下一帧重新渲染 UI

### 5. 状态管理 (`crates/state/src/`)

拆分为模块：`batch.rs`、`layout.rs`、`operations.rs`、`pending_action.rs`、`ui.rs`。

`AppState` 包含：
- 主题配置
- 终端尺寸
- 文件系统监视器
- 批量操作状态
- 模态框状态
- UI 状态（菜单、子菜单、拖拽）

### 6. 国际化 (`crates/i18n/`)

支持 15 种语言（en、ru、de、es、fr、pt、ja、ko、zh、bn、hi、id、th、tr、vi）；词典为 `crates/i18n/i18n/*.toml` 中的 TOML 文件。`Translation` trait 及其实现 `RuntimeTranslation` 提供类型安全的字符串访问。

**英文回退机制。** 如果当前语言缺少某个键，`RuntimeTranslation` 会自动返回英文值并在日志中输出警告：

```
WARN Missing translation key: my_key (using English fallback)
```

这能避免当翻译不完整时 UI 显示空字符串。对于非英文语言，英文词典总是与主词典一同加载。

**添加新键：**
1. 在 `Translation` trait (`crates/i18n/src/lib.rs`) 中添加方法。
2. 在 `RuntimeTranslation` (`crates/i18n/src/runtime.rs`) 中实现——通常是 `self.get_string("key")`，或对于带占位符的格式使用 `self.format("key", &[...])`。
3. 在 15 个 `*.toml` 文件中添加条目（缺失的会使用回退，但建议直接翻译）。

**删除键：** 从 trait 中移除方法、从 runtime 中移除实现、从所有 15 个词典中删除条目。可通过比较 `en.toml` 与其它语言的键来验证同步。

## 编码规范

### 代码风格

- 遵循 Rust 标准风格（由 `cargo fmt` 强制执行）
- 使用有意义的变量名
- 保持函数专注且简短
- 为复杂逻辑添加注释

### 错误处理

- 使用 `anyhow::Result` 进行错误传播
- 使用 `.context()` 或 `.with_context()` 添加错误上下文
- 避免 `.unwrap()` - 使用带描述性消息的 `.expect()` 或正确的错误处理
- 将错误记录到 `state.log_error()` 以便调试

### UI 代码

- 使用 `ratatui` 组件进行渲染
- 将渲染逻辑与业务逻辑分离
- 仔细计算尺寸（考虑边框、内边距）
- 在不同终端尺寸下测试 UI

### 面板实现

创建新面板时：

1. 实现 `Panel` trait
2. 在 `handle_key()` 中处理键盘输入
3. 在 `handle_mouse()` 中处理鼠标输入
4. 在 `render()` 中实现适当的渲染
5. 返回有意义的 `title()` 作为面板标题
6. 在 `app/mod.rs` 或菜单中添加面板创建逻辑

## 测试

### 手动测试清单

进行更改时，请测试：
- [ ] 不同的终端尺寸（操作过程中调整大小）
- [ ] 键盘导航（所有快捷键）
- [ ] 鼠标交互（点击、滚动）
- [ ] 模态对话框（打开、关闭、交互）
- [ ] 面板管理（打开、关闭、堆叠、拆分）
- [ ] 主题切换
- [ ] 英文和俄文 UI

### 常见问题

**面板渲染异常：**
- 检查边框计算
- 验证 area.width/height 是否考虑了边框（减去 2）
- 在最小宽度（80 字符）下测试

**焦点问题：**
- 验证 FocusTarget 是否正确更新
- 检查事件处理器中的焦点处理
- 测试空组的导航

**内存泄漏：**
- 确保面板关闭时正确释放
- 检查循环引用
- 使用 `cargo clippy` 监控

## 贡献流程

1. **Fork** 仓库
2. **创建分支** 用于您的功能/修复
3. **进行更改** 遵循编码规范
4. **充分测试**（参见上面的清单）
5. **运行代码质量检查：**
   ```bash
   cargo fmt
   cargo clippy
   cargo test
   ```
6. **提交** 带有清晰、描述性的提交消息
7. **推送** 到您的 fork
8. **创建 Pull Request** 包含：
   - 清晰的变更描述
   - 变更的原因
   - 测试结果
   - UI 变更的截图

## 调试

### 日志

TermIDE 将日志写入：
- Linux: `~/.config/termide/termide.log`
- macOS: `~/Library/Application Support/termide/termide.log`
- Windows: `%APPDATA%\\termide\\termide.log`

在代码中使用日志：
```rust
state.log_info("Info message");
state.log_error(format!("Error: {}", error));
state.log_debug("Debug message");
```

### 日志面板

使用 `Alt+L` 打开：
- 显示应用程序状态
- 显示最近的日志条目
- 显示面板信息
- 用于开发调试

### 常见调试任务

**面板未渲染：**
1. 检查面板是否在组中：`layout_manager.panel_groups`
2. 验证焦点是否正确：`layout_manager.focus`
3. 检查渲染区域是否非零

**键盘输入无效：**
1. 检查是否有模态框打开（会捕获输入）
2. 验证面板是否有焦点
3. 检查键位翻译（西里尔文支持）

**内存使用量增长：**
1. 使用 `valgrind` 或类似工具运行
2. 检查无限增长的集合
3. 验证面板关闭时是否被释放

## 性能考虑

### 渲染

- 最小化 `render()` 中的昂贵操作
- 尽可能缓存计算值
- 使用 `area` 尺寸限制工作量
- 如需可使用 `cargo flamegraph` 进行性能分析

### 文件操作

- 适当时使用异步操作
- 对文件系统事件实施防抖
- 限制目录遍历深度
- 优雅处理大文件（100 MB 限制）

### 终端操作

- 批量进行终端写入
- 最小化屏幕重绘
- 尽可能使用部分更新

## 资源

- **Ratatui:** https://github.com/ratatui-org/ratatui
- **Crossterm:** https://github.com/crossterm-rs/crossterm
- **Tree-sitter:** https://tree-sitter.github.io/
- **Rust Book:** https://doc.rust-lang.org/book/

## 获取帮助

- **Issues:** https://github.com/termide/termide/issues
- **讨论：** 使用 GitHub Discussions 提问
- **代码审查：** 在您的 PR 上请求审查

## 许可证

TermIDE 基于 MIT 许可证授权。参与贡献即表示您同意以相同条款授权您的贡献。
