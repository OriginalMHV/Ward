# TUI Dashboard

Ward includes an interactive terminal dashboard for browsing repositories, reviewing security state, and applying changes without memorizing CLI flags.

```bash
ward tui
```

---

## Tabs

The TUI has four tabs, switched with number keys:

| Key | Tab | Content |
|-----|-----|---------|
| `1` | Repos | Repository list with detail panel |
| `2` | Security | Security feature matrix across all repos |
| `3` | Actions | Available keybindings for the current context |
| `?` | Help | Full keybinding reference and filter syntax |

---

## Repos tab

The left panel shows a scrollable list of repositories with status indicators:

- `[ok]` (green) -- core security features are enabled and dependency graph / SBOM data is available
- `[!!]` (yellow) -- one or more security features are missing, or dependency graph / SBOM data needs attention
- `[..]` (gray) -- still loading

The right panel shows detail for the selected repo: name, description, language, default branch, visibility, a breakdown of each security feature (`[Y]` / `[N]`), and dependency graph / SBOM status with the current reason from GitHub.

Archived repos are marked with `[ARCHIVED]`.

---

## Security tab

A table showing security feature status across all repos in the current system:

| Column | Meaning |
|--------|---------|
| Repository | Repo name (prefix stripped if all repos share one) |
| Dependabot | Dependabot alerts enabled |
| Secret Scanning | Secret scanning enabled |
| AI Detection | AI-powered detection enabled |
| Push Protection | Push protection enabled |
| Security Updates | Dependabot security updates enabled |
| SBOM | Dependency graph / SBOM audit result (`[Y]` available, `[-]` empty, `[N]` unavailable, `[?]` unknown) |

Each cell shows `[Y]`, `[N]`, `[-]`, or `[?]` depending on the audit result.

A summary line at the bottom shows how many repos are healthy and how many need attention.

### Prefix stripping

When all repos in a system share the same name prefix (e.g., `backend-`), the security tab strips that prefix from display names to reduce visual noise. A note in the title bar indicates when stripping is active: `(prefix backend- stripped)`.

---

## Keybindings

### Navigation

| Key | Action |
|-----|--------|
| `j` / Down arrow | Move down in list |
| `k` / Up arrow | Move up in list |
| `Enter` / `l` | Load repos for current system |
| `Tab` / `s` | Cycle to next system |
| `Shift+Tab` | Cycle to previous system |
| `1` / `2` / `3` / `?` | Switch tabs |
| `q` | Quit |

### Filtering

| Key | Action |
|-----|--------|
| `/` | Enter filter mode |
| type characters | Build filter string |
| Backspace | Remove last character |
| Enter | Confirm filter and exit filter mode |
| Esc | Clear filter |

### Repo actions (on selected repo)

| Key | Action |
|-----|--------|
| `a` | Apply security settings |
| `p` | Apply branch protection |
| `t` | Deploy template (opens sub-menu) |
| `S` | Apply settings (copilot ruleset + instructions) |

When you press `t`, a sub-menu appears:

| Key | Template |
|-----|----------|
| `d` | Dependabot |
| `c` | CodeQL |
| `s` | Dependency submission |

### Bulk actions

| Key | Action |
|-----|--------|
| `A` | Apply security to all filtered repos |
| `r` | Reload repos (uses cache) |
| `R` | Force reload (clears cache) |

---

## Filter syntax

Filters are entered after pressing `/`. Terms are separated by spaces.

| Pattern | Meaning |
|---------|---------|
| `foo` | Show repos containing "foo" |
| `!ops` | Hide repos containing "ops" |
| `!ops !system` | Hide repos matching "ops" OR "system" |
| `foo !ops` | Show repos containing "foo", hide those containing "ops" |

Exclusion terms (prefixed with `!`) stack. This is useful for hiding operations and gitops repos while working on application repos:

```
!operations !gitops !system
```

Multiple inclusion terms also work -- a repo must match at least one inclusion term (if any are specified) and must not match any exclusion term.

---

## System cycling

Press `Tab` or `s` to cycle forward through systems defined in `ward.toml`. Press `Shift+Tab` to cycle backward. The current system name is shown in the header.

After switching systems, press `Enter` or `l` to load repos for the new system.

---

## Applying changes from the TUI

The TUI supports applying changes directly:

1. Navigate to a repo with `j`/`k`
2. Press one of the action keys (`a`, `p`, `t`, `S`)
3. Ward runs the corresponding apply command
4. The status indicator updates to reflect the new state

For bulk operations, filter your repo list first (e.g., `!ops` to exclude operations repos), then press `A` to apply security settings to all visible repos.
