# 键盘快捷键

Termide 在与绑定匹配前对每个按键事件进行规范化。规范化在派发边界
执行一次,其结果与原始事件一起作为 `KeyChord` 通过面板管道向下传递。
面板和模态对话框根据需要选择形式:

- **`canonical`**(规范) — 用于快捷键匹配、Vim 命令解释、设置中
  的按键捕获。
- **`raw`**(原始) — 用于文本输入(`InsertChar`)、终端面板的
  PTY 透传、搜索缓冲区输入。

在编辑器中输入的文本或发送给终端面板内程序的文本**永远不会**被
规范化重写,西里尔字母、移位字符和 locale 相关字符原样到达目标。

## 规范化修复的问题

| 问题 | 行为 |
| --- | --- |
| 西里尔字母与拉丁字母位于同一物理键(`й`/`q`、`ь`/`m`、…) | 映射到拉丁,使绑定 `Alt+M` 在 QWERTY 与 ЙЦУКЕН 布局下都生效。 |
| `REPORT_ALTERNATE_KEYS` 移位符号重写 | crossterm 将 `Shift+Ctrl+=` 重写为 `Char('+') + Ctrl`(Shift 被剥离,字符被替换)。规范化器逆向: `Char('+') + Ctrl` → `Char('=') + Ctrl + Shift`。 |
| Caps Lock 在字母上的伪 Shift | 当 `REPORT_EVENT_TYPES` 标记了 `KeyEventState::CAPS_LOCK` 时,在匹配前丢弃字母上的 Shift 位。 |
| VTE `Ctrl+/` 折叠为 `Ctrl+7` | 仅当 Kitty 协议**未**激活: `Ctrl+7` → `Ctrl+/`。 |

## 通用层 vs 增强层

某些和弦无法被所有终端编码。Termide 将默认值分为两层,如果当前
终端无法传递任何已配置的增强层和弦,会在启动时发出警告。

### 通用层(在任何 VT100+ 终端上工作)

- `Alt+字母`, `Ctrl+字母`(字母 → ASCII 控制码 0x01–0x1A)。
- `F1`–`F12` 和带**单个**修饰符的 `F1`–`F12`(`Shift+F*`、
  `Alt+F*`、`Ctrl+F*`)。
- 方向键带**单个**修饰符(`Shift+Up`、`Ctrl+Up`、`Alt+Up`)。
- `Home`、`End`、`PgUp`、`PgDn` + 单个修饰符。
- `Enter`、`Tab`、`Esc`、`Backspace`、`Delete`、`Insert` + 单个修饰符。
- `Alt+数字`。
- `Alt+标点`(`Alt+/`、`Alt+,`、`Alt+.`、…)。

### 增强层(需要 Kitty 键盘协议)

- `Ctrl+标点`(`Ctrl+/`、`Ctrl+-`、`Ctrl+=`、`Ctrl+,`、`Ctrl+.`)。
- `Ctrl+Shift+字母`。
- `Ctrl+Alt+任意键`。
- `Alt+Shift+字母` 和 `Alt+Shift+方向键` —— VTE 在传统模式下
  对 `Alt+Shift+l` 发出 `\eL`,与 `Alt+L` 无法区分;
  `Alt+Shift+...` 绑定不能匹配。
- `Super` / `Meta` / `Hyper` 修饰符。

termide 自带的增强层默认值(`toggle_comment` 和 `switch_directory` 的
`Ctrl+/`,`replace_all` 的 `Ctrl+Alt+R`)被保留,因为它们是编辑器
中的事实标准。在不支持 Kitty 协议的终端上,termide 在启动时记录
警告,列出受影响的绑定;用户可通过设置 → 键绑定重新绑定。

## 终端兼容性 (2026)

| 终端 | Kitty 键盘协议 |
| --- | --- |
| kitty | 完整 |
| foot 1.13+ | 完整 |
| WezTerm | 完整 |
| Ghostty | 完整 |
| iTerm2 | 完整 |
| rio | 完整 |
| Windows Terminal Preview 1.25+ | 完整 |
| alacritty | 部分(CSI-u,无增强标志) |
| xterm | 部分(需手动配置) |
| GNOME Terminal / Tilix / VTE | 无(开发中) |
| Konsole | 无(已计划) |
| tmux | 透传(取决于宿主终端) |

如果您的终端不公布 Kitty 协议而您依赖增强层和弦,可以切换到支持
的终端,或在 `config.toml` → `[*.keybindings]` 中将相关动作重新
绑定到通用层备选项。

## 冲突检测

设置 → 键绑定显示内联警告,当您分配的和弦已被另一个动作占用。检测
三类冲突:

- **同一节内** — 同一节中两个动作共享和弦;第二个变得无法到达。
- **跨节遮蔽** — 全局和弦遮蔽面板本地的;面板绑定永远不会触发。
- **跨节并存** — 两个面板本地绑定重叠;只有焦点面板处理事件,
  通常安全但值得注意。

同一节冲突也会在启动时记录。

## 自定义默认值

在 `config.toml` 中覆盖任何绑定。字符串以规范形式解析,因此
`"Alt++"` ≡ `"Alt+Shift+="`,`"Ctrl+Й"` ≡ `"Ctrl+Q"`:

```toml
[general.keybindings]
panel_grow_vertical = "Alt+Shift+="
panel_shrink_vertical = "Alt+Shift+-"
open_sessions = "Alt+\\"

[editor.keybindings]
trigger_completion = ["Ctrl+J", "Ctrl+Space"]
toggle_comment = ["Ctrl+/", "Ctrl+."]
replace_all = ["Ctrl+Alt+R", "Alt+R"]

[file_manager.keybindings]
switch_directory = "Ctrl+\\"

[terminal.keybindings]
switch_directory = "Ctrl+\\"
```

注:`Ctrl+/` 和 `Ctrl+\` 通过 `KeyNormalizer` quirk 即使在传统终端
(例如 VTE)上也能工作 —— VTE 将它们作为 `\x1F` 和 `\x1C` 控制字节发送,
crossterm 解析为 `Ctrl+7` / `Ctrl+4`,规范化器将其重写回斜杠 / 反斜杠。

任何动作都支持多个备选项:列在数组中。第一项是帮助面板中显示的
规范字符串。
