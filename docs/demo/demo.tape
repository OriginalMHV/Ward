# Records docs/assets/demo.gif. Requires: vhs, ward on PATH, GH_TOKEN set.
# Run docs/demo/reset.sh before each recording.
Output docs/assets/demo.gif

Set Shell "bash"
Set FontSize 18
Set Width 1200
Set Height 700
Set Padding 28
Set TypingSpeed 35ms
Set Framerate 24
Set PlaybackSpeed 1
Set Theme { "name": "Ward Light", "black": "#0F2A44", "red": "#B42318", "green": "#0F766E", "yellow": "#B54708", "blue": "#1D4ED8", "magenta": "#7E22CE", "cyan": "#14B8A6", "white": "#CBD5E1", "brightBlack": "#64748B", "brightRed": "#DC2626", "brightGreen": "#14B8A6", "brightYellow": "#D97706", "brightBlue": "#2563EB", "brightMagenta": "#9333EA", "brightCyan": "#0D9488", "brightWhite": "#F7F5F0", "background": "#F7F5F0", "foreground": "#0F2A44", "selection": "#CCEFEA", "cursor": "#14B8A6" }

Hide
Type `export PS1='$ ' && cd $(mktemp -d) && clear`
Enter
Show

# 1. Import the current state of a repository as the baseline.
Type "ward import OriginalMHV/ward-demo --output ward.toml"
Enter
Sleep 5s

# 2. Look at the repository settings.
Type `grep -n -E 'delete_branch_on_merge|^topics' ward.toml`
Enter
Sleep 1500ms

# 3. Change the desired state.
Type `sed -i.bak -e 's/delete_branch_on_merge = false/delete_branch_on_merge = true/' -e 's/^topics = .*/topics = ["managed-by-ward"]/' ward.toml`
Enter
Sleep 1s
Type `grep -n -E 'delete_branch_on_merge|^topics' ward.toml`
Enter
Sleep 2500ms
Type "clear"
Enter

# 4. Plan shows the drift.
Type "ward plan --category repository"
Enter
Sleep 6s
Type "clear"
Enter

# 5. Apply fixes it.
Type "ward apply --category repository --yes"
Enter
Sleep 7s
Type "clear"
Enter

# 6. Drift check passes.
Type `ward drift && echo "exit code: $?"`
Enter
Sleep 5s

Hide
Type "cd ~ && clear"
Enter
Show
Sleep 1s
