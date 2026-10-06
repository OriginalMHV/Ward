# Records docs/assets/demo.gif. Requires: vhs, ward on PATH, GH_TOKEN set.
# Run docs/demo/reset.sh before each recording.
Output docs/assets/demo.gif

Set Shell "bash"
Set Theme { "name": "Ward", "background": "#0B1F44", "foreground": "#E6EDF8", "cursor": "#93C5FD", "selection": "#1E40AF", "black": "#0B1F44", "red": "#F87171", "green": "#86EFAC", "yellow": "#FCD34D", "blue": "#60A5FA", "magenta": "#C4B5FD", "cyan": "#93C5FD", "white": "#E6EDF8", "brightBlack": "#5B7AAE", "brightRed": "#FCA5A5", "brightGreen": "#BBF7D0", "brightYellow": "#FDE68A", "brightBlue": "#93C5FD", "brightMagenta": "#DDD6FE", "brightCyan": "#BFDBFE", "brightWhite": "#FFFFFF" }
Set WindowBar Colorful
Set WindowBarSize 40
Set BorderRadius 16
Set Margin 40
Set MarginFill "#2563EB"
Set Padding 28
Set FontFamily "JetBrains Mono"
Set FontSize 22
Set Width 1200
Set Height 560
Set LineHeight 1.35
Set TypingSpeed 45ms
Set Framerate 24

Hide
Type `export PS1='$ ' HOME=$(mktemp -d) && cd $HOME && clear`
Enter
Show

Hide
Type `clear; printf '\033[1;38;2;147;197;253m%s\033[0m\n' '# 1. Import a repository as the baseline'`
Enter
Sleep 300ms
Show
Type "ward import OriginalMHV/ward-demo --output ward.toml"
Enter
Sleep 8s
Sleep 3s

Hide
Type `clear; printf '\033[1;38;2;147;197;253m%s\033[0m\n' '# 2. Change the desired state'`
Enter
Sleep 300ms
Show
Type `grep -n -E 'delete_branch_on_merge|^topics' ward.toml`
Enter
Sleep 2s
Hide
Type `sed -i.bak -e 's/delete_branch_on_merge = false/delete_branch_on_merge = true/' -e 's/^topics = .*/topics = ["managed-by-ward"]/' ward.toml`
Enter
Sleep 500ms
Show
Type `grep -n -E 'delete_branch_on_merge|^topics' ward.toml`
Enter
Sleep 4s

Hide
Type `clear; printf '\033[1;38;2;147;197;253m%s\033[0m\n' '# 3. Preview the drift'`
Enter
Sleep 300ms
Show
Type "ward plan --category repository"
Enter
Sleep 2s
Sleep 3s

Hide
Type `clear; printf '\033[1;38;2;147;197;253m%s\033[0m\n' '# 4. Apply and verify'`
Enter
Sleep 300ms
Show
Type "ward apply --category repository --yes"
Enter
Sleep 4s
Sleep 3s

Hide
Type `clear; printf '\033[1;38;2;147;197;253m%s\033[0m\n' '# 5. CI drift check passes'`
Enter
Sleep 300ms
Show
Type `ward drift --category repository; echo "exit code: $?"`
Enter
Sleep 4s
Sleep 3s
