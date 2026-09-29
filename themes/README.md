# SCAS editor themes

The editor ships with a broad set of familiar developer palettes: Dark Modern,
Light Modern, One Dark Pro, Dracula, Tokyo Night, Catppuccin Mocha, Nord,
GitHub Dark, Solarized Dark, Monokai, Gruber Darker, and Paper Light. Open
**Settings**, choose a color scheme, or enter a theme file path and choose
**Load** to add another scheme for the current session.

Themes are TOML files. Every field is required, and colors accept `#RRGGBB` or
`#RRGGBBAA`:

```toml
name = "My Theme"
dark = true

[ui]
background = "#181818"
panel = "#202020"
editor = "#181818"
text = "#e4e4ef"
muted = "#95a99f"
accent = "#ffdd33"
selection = "#484848"

[syntax]
plain = "#e4e4ef"
keyword = "#ffdd33"
number = "#de935f"
string = "#73c936"
comment = "#95a99f"
doc_comment = "#9e95c7"
identifier = "#e4e4ef"
operator = "#ffdd33"
delimiter = "#cc8c3c"
render_marker = "#96a6c8"
```

`doc_comment` colors `///` document prose. `render_marker` colors `\{`, `\}`,
`\[` and `\]` independently from ordinary brackets and braces.
