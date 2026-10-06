# Records docs/assets/demo.gif. Requires: vhs, ward on PATH, GH_TOKEN set.
# Run docs/demo/reset.sh before each recording.
Output docs/assets/demo.gif

Set Shell "bash"
Set Theme { "name": "Ward Light", "background": "#FFFFFF", "foreground": "#1F2328", "cursor": "#0969DA", "selection": "#DDF4FF", "black": "#1F2328", "red": "#CF222E", "green": "#0969DA", "yellow": "#0550AE", "blue": "#0969DA", "magenta": "#59636E", "cyan": "#0550AE", "white": "#59636E", "brightBlack": "#8C959F", "brightRed": "#A40E26", "brightGreen": "#0550AE", "brightYellow": "#0550AE", "brightBlue": "#0550AE", "brightMagenta": "#59636E", "brightCyan": "#0550AE", "brightWhite": "#1F2328" }
Set WindowBar Colorful
Set WindowBarSize 40
Set BorderRadius 14
Set Margin 16
Set MarginFill "#F6F8FA"
Set Padding 28
Set FontFamily "JetBrains Mono"
Set FontSize 22
Set Width 1300
Set Height 700
Set LineHeight 1.35
Set TypingSpeed 25ms
Set Framerate 24

Hide
Type `export PS1='$ ' HOME=$(mktemp -d) && cd $HOME && clear`
Enter
Show

Hide
Type `clear; printf '\033[1;38;2;9;105;218m%s\033[0m\n' '# 1. Import a repository as the baseline'`
Enter
Sleep 300ms
Show
Type "ward import OriginalMHV/ward-demo --output ward.toml"
Enter
Sleep 1500ms
Hide
Sleep 7s
Show
Sleep 2500ms

Hide
Type `sed -i.bak -e 's/delete_branch_on_merge = false/delete_branch_on_merge = true/' -e 's/^topics = .*/topics = ["managed-by-ward"]/' ward.toml`
Enter
Sleep 500ms
Type `clear; printf '\033[1;38;2;9;105;218m%s\033[0m\n' '# 2. Change the desired state'`
Enter
Sleep 300ms
Show
Type `grep -n -E 'delete_branch_on_merge|^topics' ward.toml`
Enter
Sleep 2500ms

Hide
Type `clear; printf '\033[1;38;2;9;105;218m%s\033[0m\n' '# 3. Preview the drift'`
Enter
Sleep 300ms
Show
Type "ward plan --category repository"
Enter
Sleep 1500ms
Sleep 3s

Hide
Type `clear; printf '\033[1;38;2;9;105;218m%s\033[0m\n' '# 4. Apply and verify'`
Enter
Sleep 300ms
Show
Type "ward apply --category repository --yes"
Enter
Sleep 1s
Hide
Sleep 3s
Show
Sleep 3s

Hide
Type `clear; printf '\033[1;38;2;9;105;218m%s\033[0m\n' '# 5. CI drift check passes'`
Enter
Sleep 300ms
Show
Type `ward drift --category repository; echo "exit code: $?"`
Enter
Sleep 1s
Hide
Sleep 3s
Show
Sleep 3500ms
