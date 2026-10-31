# Third-party notice

The PDF plugin's engine for annotations, page marks, page organisation and
text extraction is **PdfCraft** (https://github.com/storytold/pdfcraft),
used unmodified as Cargo git dependencies pinned to commit
`f137a289baadefbd29e0e46930899896068cc41d`:

`pdfcraft-cos`, `pdfcraft-annot`, `pdfcraft-edit`, `pdfcraft-organize`,
`pdfcraft-render` (and their workspace dependencies), plus PdfCraft's patched
forks of `hayro`, `vello_cpu`/`vello_common` and `lopdf` (see `[patch]` in the
workspace `Cargo.toml`).

Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.
Licensed under either of the MIT license or the Apache License 2.0, at your
option. The license texts and PdfCraft's own NOTICE and ATTRIBUTION files are
in its repository at that commit.
