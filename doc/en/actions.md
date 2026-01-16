# Custom Actions

The Actions system allows you to add custom scripts to TermIDE's menu bar. Scripts are executed in a new terminal panel, making it easy to run build commands, deployment scripts, or any automation tasks directly from TermIDE.

## Getting Started

### Actions Directory Location

Place your scripts in the actions directory:

| Platform | Path |
|----------|------|
| Linux | `~/.config/termide/actions/` |
| macOS | `~/Library/Application Support/termide/actions/` |
| Windows | `%APPDATA%\termide\actions\` |

You can also access this folder via `Options → Manage actions` in the menu bar.

### Creating Your First Action

```bash
# Create the actions directory
mkdir -p ~/.config/termide/actions

# Create a simple script
cat > ~/.config/termide/actions/hello.sh << 'EOF'
#!/bin/bash
echo "Hello from TermIDE!"
echo "Current directory: $(pwd)"
read -p "Press Enter to close..."
EOF

# Make it executable (required on Unix)
chmod +x ~/.config/termide/actions/hello.sh
```

After creating the script, restart TermIDE or use `Options → Manage actions` to refresh. Your script will appear in the **Actions** menu.

## Script Naming

The display name in the menu is derived from the filename:

| Filename | Display Name |
|----------|--------------|
| `build.sh` | build |
| `deploy.sh` | deploy |
| `run-tests.py` | run-tests |
| `my.cool.script.sh` | my |

The display name is the part of the filename before the first dot.

## Directory Structure (Groups)

You can organize scripts into groups using subdirectories. Each subdirectory becomes a submenu:

```
~/.config/termide/actions/
├── build.sh              # Appears in Actions menu root
├── deploy.sh             # Appears in Actions menu root
├── docker/               # Creates "docker" submenu
│   ├── up.sh
│   ├── down.sh
│   └── logs.sh
└── git/                  # Creates "git" submenu
    ├── pull.sh
    ├── push.sh
    └── status.sh
```

**Note:** Only one level of subdirectories is supported. Nested subdirectories are ignored.

## Background Execution

By default, scripts run in a foreground terminal panel. For long-running processes that you want to run in the background, add `.bg.` to the filename:

| Filename | Execution Mode |
|----------|----------------|
| `server.sh` | Foreground (new terminal panel) |
| `server.bg.sh` | Background |
| `deploy.bg.sh` | Background |

Background scripts run without opening a terminal panel, useful for:
- Starting development servers
- Running watch processes
- Launching background services

## Platform-Specific Notes

### Unix (Linux/macOS)

Scripts must have the executable permission:

```bash
chmod +x ~/.config/termide/actions/myscript.sh
```

Any file with the executable bit set will appear in the menu, regardless of extension.

### Windows

On Windows, the following file extensions are recognized as executable:
- `.sh` (requires WSL or Git Bash)
- `.bat`
- `.cmd`
- `.ps1` (PowerShell)
- `.py` (Python)
- `.rb` (Ruby)
- `.pl` (Perl)

## Working Directory

Scripts are executed with the current session's root directory as the working directory. This is typically the directory where you launched TermIDE or the directory you selected via `Sessions → Change root`.

## Tips

1. **Add shebang line**: Always start scripts with a shebang (e.g., `#!/bin/bash`) to ensure they run with the correct interpreter.

2. **Keep output visible**: For foreground scripts, add `read -p "Press Enter..."` at the end to see output before the terminal closes.

3. **Use descriptive names**: The filename before the first dot becomes the menu label, so use clear, descriptive names.

4. **Organize with groups**: Use subdirectories to group related scripts (e.g., `docker/`, `npm/`, `git/`).

5. **Background for servers**: Use `.bg.` in the filename for scripts that start long-running processes.

## Troubleshooting

### Script doesn't appear in menu

1. Check that the file has executable permission (`chmod +x`)
2. Make sure the file is in the correct directory
3. Restart TermIDE to refresh the actions list

### Script fails to run

1. Test the script manually in a terminal first
2. Check the shebang line is correct
3. Ensure all required tools/interpreters are installed

### Background script doesn't seem to work

Background scripts run silently. Check:
1. The script actually starts the intended process
2. The process isn't immediately exiting
3. Look for process output in system logs if needed
