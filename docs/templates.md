# Templates

Ward uses [Tera](https://keats.github.io/tera/) templates (Jinja2-compatible) to generate configuration files that are committed to repositories via the Git Trees API. Templates are rendered per-repo with auto-detected context variables, then committed as pull requests.

---

## Built-in templates

Ward ships with these embedded templates:

| Template | Target file | Project type |
|----------|-------------|-------------|
| `dependabot/gradle.yml.tera` | `.github/dependabot.yml` | Gradle |
| `dependabot/npm.yml.tera` | `.github/dependabot.yml` | npm |
| `codeql/gradle.yml.tera` | `.github/workflows/codeql.yml` | Gradle |
| `codeql/npm.yml.tera` | `.github/workflows/codeql.yml` | npm |
| `dependency-submission/gradle.yml.tera` | `.github/workflows/dependency-submission.yml` | Gradle |
| `copilot-review/instructions-app.md.tera` | `.github/copilot-instructions.md` | Application repos |
| `copilot-review/instructions-ops.md.tera` | `.github/copilot-instructions.md` | Operations repos |

---

## Auto-detection logic

When you run `ward commit apply --template <name>`, Ward detects the project type and version information for each repository:

### Project type detection

Ward checks for these files in order:

1. `build.gradle.kts` -- **Gradle**
2. `build.gradle` -- **Gradle**
3. `package.json` -- **npm**
4. `Cargo.toml` -- **Cargo**
5. None found -- **Unknown**

### Version detection

- **Java version**: extracted by regex from `build.gradle` or `build.gradle.kts`
- **Node version**: parsed from the `engines` field in `package.json` (major version only)
- **Spring Boot version**: pattern matched against the `org.springframework.boot` plugin in Gradle files

### Ops repo detection

For `ward settings apply --copilot-instructions`, Ward classifies repos as operations or application repos by suffix:

- `-operation`, `-operations`, `-ops`, `-gitops` -- operations repo (uses `instructions-ops.md.tera`)
- Everything else -- application repo (uses `instructions-app.md.tera`)

---

## Template variables

Variables available inside Tera templates, populated automatically per-repo:

| Variable | Type | Default | Used in |
|----------|------|---------|---------|
| `java_version` | string | `"21"` | `codeql/gradle`, `dependency-submission/gradle` |
| `node_version` | string | `"20"` | `codeql/npm` |
| `default_branch` | string | `"main"` | `dependency-submission/gradle` |
| `registry_url` | string | `"https://repo.maven.apache.org/maven2"` | `dependabot/gradle` |
| `jfrog_oidc_provider` | string | none | `dependabot/gradle` |

The `registry_url` and `jfrog_oidc_provider` variables come from the `[templates.registries]` configuration in `ward.toml`. The version variables are auto-detected from the repo's build files.

---

## Custom templates

### Location

Custom templates live in `~/.ward/templates/` by default. Override with `custom_dir` in `ward.toml`:

```toml
[templates]
custom_dir = "/path/to/my/templates"
```

Or check the current directory:

```bash
ward template dir
```

### Override behavior

A custom template with the same path as a built-in template overrides the built-in. For example, placing a file at `~/.ward/templates/dependabot/gradle.yml.tera` replaces the built-in Dependabot Gradle template.

Use `ward template list` to see which templates are active and whether they come from the built-in set or your custom directory.

### Tera syntax basics

Tera uses `{{ }}` for expressions and `{% %}` for control flow:

```
# Variable substitution
{{ java_version }}

# Default values
{{ java_version | default(value="21") }}

# Conditionals
{% if jfrog_oidc_provider %}
    jfrog-oidc-provider-name: '{{ jfrog_oidc_provider }}'
{% endif %}

# Comments
{# This is a comment and won't appear in output #}
```

See the [Tera documentation](https://keats.github.io/tera/docs/) for the full syntax reference.

---

## Template management commands

```bash
ward template list                              # list all templates with source
ward template show codeql/gradle.yml.tera       # view template content
ward template export                            # export all built-ins to ~/.ward/templates/
ward template export dependabot/gradle.yml.tera # export one for customization
ward template create my-team/custom.yml.tera    # create a new custom template
ward template dir                               # show/create custom templates dir
```

---

## Registries configuration for Gradle Artifactory

If your organization uses a private Maven repository (e.g., JFrog Artifactory), configure it in the `[templates.registries]` section. This feeds into the Dependabot Gradle template:

```toml
[templates.registries.gradle-artifactory]
type = "maven-repository"
url = "https://your-artifactory.example.com/artifactory/maven"
jfrog_oidc_provider = "your-oidc-provider-id"
```

This generates a `registries` block in the Dependabot config, telling Dependabot to check your private registry for dependency updates.

---

## Example: creating a custom template from scratch

1. Create the template file:

```bash
ward template create my-org/pr-template.md.tera
```

2. Edit the file at `~/.ward/templates/my-org/pr-template.md.tera`:

```markdown
## Pull Request

### Changes
<!-- Describe your changes -->

### Checklist
- [ ] Tests pass
- [ ] Documentation updated
- [ ] No secrets committed
```

3. Deploy it:

```bash
ward commit plan --template my-org/pr-template --system backend
ward commit apply --template my-org/pr-template --system backend
```

---

## Example: customizing a built-in template

1. Export the built-in template you want to modify:

```bash
ward template export dependabot/gradle.yml.tera
```

2. Edit `~/.ward/templates/dependabot/gradle.yml.tera` to add your changes. For example, add an extra package ecosystem:

```yaml
updates:
  - package-ecosystem: gradle
    directory: /
    schedule:
      interval: weekly
  - package-ecosystem: docker
    directory: /
    schedule:
      interval: weekly
```

3. Verify the override is active:

```bash
ward template list
# Should show dependabot/gradle.yml.tera as "override" source
```

4. Deploy:

```bash
ward commit apply --template dependabot --system backend
```

The customized template is now used instead of the built-in one.
