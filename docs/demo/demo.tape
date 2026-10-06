# Records docs/assets/demo.gif. Requires: vhs, ward on PATH, GH_TOKEN set.
# Run docs/demo/reset.sh before each recording.
Output docs/assets/demo.gif

Set Shell "bash"
Set Theme { "name": "Ward Ink", "background": "#0F172A", "foreground": "#E2E8F0", "cursor": "#F5A524", "selection": "#334155", "black": "#0F172A", "red": "#F87171", "green": "#FCD48A", "yellow": "#F5A524", "blue": "#A8B3C5", "magenta": "#CBD5E1", "cyan": "#FCD48A", "white": "#E2E8F0", "brightBlack": "#7C8AA0", "brightRed": "#FCA5A5", "brightGreen": "#FDE6B4", "brightYellow": "#FCD48A", "brightBlue": "#CBD5E1", "brightMagenta": "#E2E8F0", "brightCyan": "#FDE6B4", "brightWhite": "#FFFFFF" }
Set WindowBar Colorful
Set WindowBarSize 40
Set BorderRadius 14
Set Margin 16
Set MarginFill "#0A101F"
Set Padding 28
Set FontFamily "JetBrains Mono"
Set FontSize 22
Set Width 1300
Set Height 700
Set LineHeight 1.35
Set TypingSpeed 45ms
Set Framerate 24

Hide
Type `export PS1='$ ' HOME=$(mktemp -d) && cd $HOME && clear`
Enter
Show

Hide
Type `clear; printf '\033[1;38;2;245;165;36m%s\033[0m\n' '# 1. Import a repository as the baseline'`
Enter
Sleep 300ms
Show
Type "ward import OriginalMHV/ward-demo --output ward.toml"
Enter
Sleep 8s
Sleep 3s

Hide
Type `clear; printf '\033[1;38;2;245;165;36m%s\033[0m\n' '# 2. Change the desired state'`
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
Type `clear; printf '\033[1;38;2;245;165;36m%s\033[0m\n' '# 3. Preview the drift'`
Enter
Sleep 300ms
Show
Type "ward plan --category repository"
Enter
Sleep 2s
Sleep 3s

Hide
Type `clear; printf '\033[1;38;2;245;165;36m%s\033[0m\n' '# 4. Apply and verify'`
Enter
Sleep 300ms
Show
Type "ward apply --category repository --yes"
Enter
Sleep 4s
Sleep 3s

Hide
Type `clear; printf '\033[1;38;2;245;165;36m%s\033[0m\n' '# 5. CI drift check passes'`
Enter
Sleep 300ms
Show
Type `ward drift --category repository; echo "exit code: $?"`
Enter
Sleep 4s
Sleep 3s
