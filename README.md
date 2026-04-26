# Defold Kit (Zed)

Defold API and native extension annotations injected into the Lua language server in [Zed](https://zed.dev).

This is a Zed port of the code-intelligence parts of [astrochili/vscode-defold](https://github.com/astrochili/vscode-defold) by Roman Silin. Build/launch/debug/bundle features are not included — this extension is purely about working with Lua code in Defold projects.

> This repo is a fork of [astrochili/vscode-defold](https://github.com/astrochili/vscode-defold). All original VS Code source has been replaced with a Zed-specific implementation; the fork relationship is preserved on GitHub as attribution to the original author.

This extension does **not** ship its own language or LSP. It piggy-backs on Zed's official **Lua** extension via the `language_server_additional_workspace_configuration` callback, injecting `Lua.workspace.library` paths so `lua-language-server` resolves Defold APIs and the APIs of native extensions in `.internal/lib`.

## Requirements

- Zed
- Zed's official **Lua** extension (provides `lua-language-server` for Lua files)
- macOS or Linux (Windows is untested — `find`/`unzip`/`cat` need to be on `PATH`)

## Installation

Until published to the Zed extension registry:

1. Clone this repository
2. In Zed: `zed: install dev extension` → pick the cloned folder

## Setup

Add this once to your Zed settings (project's `.zed/settings.json` or global `~/.config/zed/settings.json`) so `.script` / `.gui_script` / `.render_script` / `.editor_script` files are treated as Lua:

```json
{
  "file_types": {
    "Lua": ["script", "gui_script", "render_script", "editor_script"]
  }
}
```

That's it — open any file in a Defold project (one with a `game.project` at the root) and Defold completions will be available.

## Configuration

Optional. By default, Defold annotations are pulled from the latest [astrochili/defold-annotations](https://github.com/astrochili/defold-annotations) release.

To pin annotations to your installed Defold version, point the extension at the editor:

```json
{
  "lsp": {
    "lua-language-server": {
      "settings": {
        "defold_kit": {
          "editor_path": "/Applications/Defold.app"
        }
      }
    }
  }
}
```

The extension reads `<editor_path>/Contents/Resources/config` (macOS) or `<editor_path>/config` (Linux), extracts `build.version`, and downloads matching annotations.

Or specify the version directly:

```json
{
  "lsp": {
    "lua-language-server": {
      "settings": {
        "defold_kit": { "version": "1.9.0" }
      }
    }
  }
}
```

Resolution priority: `version` → `editor_path` → latest release.

## How it works

The extension registers an LSP adapter (`defold-lua-ls`) attached to a phantom language `Defold Script` that no file ever uses. The adapter never actually starts a process, but Zed still calls its `language_server_additional_workspace_configuration` whenever any other registered LSP — including the Lua extension's `lua-language-server` — builds its workspace configuration.

When that callback fires for `lua-language-server` and the worktree contains a `game.project`:

1. Resolves the Defold version (from `version` setting → `editor_path` → latest release)
2. Downloads `defold_api_<version>.zip` from `astrochili/defold-annotations` and unpacks it into the extension's working directory
3. Lists `<workspace>/.internal/lib/*.zip` (native-extension archives produced by Defold's *Fetch Libraries*)
4. For each archive: extracts it, reads the inner `game.project`, collects `[library] include_dirs`
5. Returns `{"Lua": {"workspace": {"library": [...absolute paths...]}}}` for Zed to merge into the LSP's workspace config

After running *Fetch Libraries* in Defold, restart the language server (`editor: restart language server`) so the new dependency annotations are picked up.

## Snippets

Triggers in any Lua file: `script` (full file template), `init(self)`, `update(self, dt)`, `fixed_update(self, dt)`, `on_message(...)`, `on_input(...)`, `final(self)`, `on_reload(self)`.

## Credits

- Based on [vscode-defold](https://github.com/astrochili/vscode-defold) by Roman Silin (MIT)
- Annotations from [astrochili/defold-annotations](https://github.com/astrochili/defold-annotations)

## License

MIT
