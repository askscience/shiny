# Third-party notice

The Word plugin's document engine (`src/engine.rs`) uses **WordCraft**
(https://github.com/storytold/wordcraft) as a library, unmodified, through
Cargo git dependencies pinned to commit
`a88e005256087285bc7d34310cc0f5f932bc8562`:

`wordcraft-doc` (document model), `wordcraft-docx` (.docx reader/writer),
`wordcraft-formats` (ODT, HTML, Markdown, RTF, plain text),
`wordcraft-layout` (pagination: contents page numbers and the preview),
`wordcraft-render` (page rasters for the window's preview),
`wordcraft-fonts` (the bundled faces the window serves) and
`wordcraft-pdf` (PDF export), with their workspace dependencies
(`wordcraft-geom`, `wordcraft-fonts`, `wordcraft-metafile`, `wordcraft-proof`)
and craft-libs' `craft-pdf` and patched `krilla`
(https://github.com/storytold/craft-libs, commit
`92d392cff132eaf53e13afff77f38b5c11b73aa0`, see `[patch]` in the workspace
`Cargo.toml`).

Copyright (c) the ArtCraft Team and the WordCraft contributors.
Licensed under either of the MIT license or the Apache License 2.0, at your
option. The license texts and WordCraft's own NOTICE and ATTRIBUTION files are
in its repository at that commit.

WordCraft bundles the Source Sans 3, Source Serif 4, Inter and JetBrains Mono
fonts (SIL Open Font License 1.1); PDF export embeds subsets of them.
