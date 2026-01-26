# Custom Scripts

The Scripts system allows you to add custom scripts to TermIDE's menu bar. Scripts are executed in a new terminal panel, making it easy to run build commands, deployment scripts, or any automation tasks directly from TermIDE.

## Getting Started

### Scripts Directory Location

Place your scripts in the scripts directory:

| Platform | Path |
|----------|------|
| Linux | `~/.local/share/termide/scripts/` |
| macOS | `~/Library/Application Support/termide/scripts/` |
| Windows | `%APPDATA%\termide\scripts\` |

You can also access this folder via `Options → Manage scripts` in the menu bar.

### Creating Your First Script

```bash
# Create the scripts directory
mkdir -p ~/.local/share/termide/scripts

# Create a simple script
cat > ~/.local/share/termide/scripts/hello.sh << 'EOF'
#!/bin/bash
echo "Hello from TermIDE!"
echo "Current directory: $(pwd)"
read -p "Press Enter to close..."
EOF

# Make it executable (required on Unix)
chmod +x ~/.local/share/termide/scripts/hello.sh
```

After creating the script, restart TermIDE or use `Options → Manage scripts` to refresh. Your script will appear in the **Scripts** menu.

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
~/.local/share/termide/scripts/
├── build.sh              # Appears in Scripts menu root
├── deploy.sh             # Appears in Scripts menu root
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

## Execution Modes

Scripts support different execution modes based on filename suffixes:

### Background Execution (`.bg.`)

For long-running processes that you want to run in the background, add `.bg.` to the filename:

| Filename | Execution Mode |
|----------|----------------|
| `server.sh` | Foreground (new terminal panel) |
| `server.bg.sh` | Background (no terminal panel) |
| `deploy.bg.sh` | Background |

Background scripts run without opening a terminal panel, useful for:
- Starting development servers
- Running watch processes
- Launching background services

### Report Scripts (`.report.`)

For scripts that should run in the background and show their output in a modal dialog, add `.report.` to the filename:

| Filename | Execution Mode |
|----------|----------------|
| `check.sh` | Foreground (new terminal panel) |
| `check.report.sh` | Background with modal output |
| `status.report.sh` | Background with modal output |

Report scripts:
- Run in the background without blocking the UI
- Capture stdout and stderr
- Display output in an informational modal when completed
- Show success (✓) or failure (✗) indicator in modal title

**Example use cases:**
- Quick status checks (`git status`, `docker ps`)
- Linting or validation scripts
- System health checks
- Any short-running script where you want to see the result

**Example:**
```bash
# Create a report script
cat > ~/.local/share/termide/scripts/check.report.sh << 'EOF'
#!/bin/bash
echo "Checking system status..."
echo "Date: $(date)"
echo "User: $(whoami)"
echo "PWD: $(pwd)"
EOF
chmod +x ~/.local/share/termide/scripts/check.report.sh
```

## Platform-Specific Notes

### Unix (Linux/macOS)

Scripts must have the executable permission:

```bash
chmod +x ~/.local/share/termide/scripts/myscript.sh
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

6. **Report for quick checks**: Use `.report.` in the filename for scripts where you want to see the result in a modal.

## Troubleshooting

### Script doesn't appear in menu

1. Check that the file has executable permission (`chmod +x`)
2. Make sure the file is in the correct directory
3. Restart TermIDE to refresh the scripts list

### Script fails to run

1. Test the script manually in a terminal first
2. Check the shebang line is correct
3. Ensure all required tools/interpreters are installed

### Background script doesn't seem to work

Background scripts run silently. Check:
1. The script actually starts the intended process
2. The process isn't immediately exiting
3. Look for process output in system logs if needed

### Report script modal doesn't appear

Report scripts show a modal when they complete:
1. Wait for the script to finish executing
2. Check if the script exits too quickly (add a small delay if needed)
3. Ensure the script produces output to stdout or stderr
