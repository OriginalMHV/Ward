# Records docs/assets/demo.gif. Requires: vhs, ward on PATH, GH_TOKEN set.
# Run docs/demo/reset.sh before each recording.
# Record to MP4 for exact colours, then convert:
#   vhs docs/demo/demo.tape -o demo.mp4
#   ffmpeg -i demo.mp4 -vf "fps=20,split[a][b];[a]palettegen=max_colors=128:stats_mode=full[p];[b][p]paletteuse=dither=none" demo.gif
#   Then mask the outer corners to transparent (radius 16) and run gifsicle -O3.
Output docs/assets/demo.gif

Set Shell "bash"
Set Theme { "name": "Ward GitHub Dark", "background": "#0D1117", "foreground": "#E6EDF3", "cursor": "#E3B341", "selection": "#3D444D", "black": "#484F58", "red": "#FF7B72", "green": "#3FB950", "yellow": "#E3B341", "blue": "#58A6FF", "magenta": "#BC8CFF", "cyan": "#39C5CF", "white": "#B1BAC4", "brightBlack": "#6E7681", "brightRed": "#FFA198", "brightGreen": "#56D364", "brightYellow": "#F0C14B", "brightBlue": "#79C0FF", "brightMagenta": "#D2A8FF", "brightCyan": "#56D4DD", "brightWhite": "#F0F6FC" }
Set WindowBar Colorful
Set WindowBarSize 40
Set BorderRadius 12
Set Margin 4
Set MarginFill "#3D444D"
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
Type `clear; printf '\033[1;38;2;227;179;65m%s\033[0m\n' '# 1. Import a repository as the baseline'`
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
Type `clear; printf '\033[1;38;2;227;179;65m%s\033[0m\n' '# 2. Change the desired state'`
Enter
Sleep 300ms
Show
Type `diff ward.toml.bak ward.toml`
Enter
Sleep 3s

Hide
Type `clear; printf '\033[1;38;2;227;179;65m%s\033[0m\n' '# 3. Preview the drift'`
Enter
Sleep 300ms
Show
Type "ward plan --category repository"
Enter
Sleep 1500ms
Sleep 3s

Hide
Type `clear; printf '\033[1;38;2;227;179;65m%s\033[0m\n' '# 4. Apply and verify'`
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
Type `clear; printf '\033[1;38;2;227;179;65m%s\033[0m\n' '# 5. CI drift check passes'`
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
