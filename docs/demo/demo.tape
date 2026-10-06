# Records docs/assets/demo.gif. Requires: vhs, ward on PATH, GH_TOKEN set.
# Run docs/demo/reset.sh before each recording.
Output docs/assets/demo.gif

Set Shell "bash"
Set FontSize 20
Set Width 1200
Set Height 780
Set Padding 24
Set Margin 20
Set MarginFill "#0F2A44"
Set BorderRadius 12
Set WindowBar Colorful
Set TypingSpeed 45ms
Set Framerate 24
Set Theme { "name": "Ward Night", "black": "#0A1C2E", "red": "#F87171", "green": "#5EEAD4", "yellow": "#FBBF24", "blue": "#7DB5FF", "magenta": "#C4B5FD", "cyan": "#14B8A6", "white": "#E6EDF3", "brightBlack": "#6F88A1", "brightRed": "#FCA5A5", "brightGreen": "#99F6E4", "brightYellow": "#FCD34D", "brightBlue": "#A5CBFF", "brightMagenta": "#DDD6FE", "brightCyan": "#2DD4BF", "brightWhite": "#FFFFFF", "background": "#0A1C2E", "foreground": "#E6EDF3", "selection": "#16385A", "cursor": "#14B8A6" }

Hide
Type `export PS1='$ ' HOME=$(mktemp -d) && cd $HOME && clear`
Enter
Show

Hide
Type `clear; printf '\033[1;38;2;20;184;166m%s\033[0m\n' '# 1. Import a repository as the baseline'`
Enter
Sleep 300ms
Show
Type "ward import OriginalMHV/ward-demo --output ward.toml"
Enter
Sleep 8s
Sleep 3s

Hide
Type `clear; printf '\033[1;38;2;20;184;166m%s\033[0m\n' '# 2. Change the desired state'`
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
Type `clear; printf '\033[1;38;2;20;184;166m%s\033[0m\n' '# 3. Preview the drift'`
Enter
Sleep 300ms
Show
Type "ward plan --category repository"
Enter
Sleep 2s
Sleep 3s

Hide
Type `clear; printf '\033[1;38;2;20;184;166m%s\033[0m\n' '# 4. Apply and verify'`
Enter
Sleep 300ms
Show
Type "ward apply --category repository --yes"
Enter
Sleep 4s
Sleep 3s

Hide
Type `clear; printf '\033[1;38;2;20;184;166m%s\033[0m\n' '# 5. CI drift check passes'`
Enter
Sleep 300ms
Show
Type `ward drift; echo "exit code: $?"`
Enter
Sleep 7s
Sleep 3s
