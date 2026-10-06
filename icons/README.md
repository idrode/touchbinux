# Icons

The SVG files in this directory are copied unchanged from
[tiny-dfr](https://github.com/AsahiLinux/tiny-dfr) (`share/tiny-dfr/`, commit
`eb711c8`), the Touch Bar daemon of the Asahi Linux project.

- They are Google's [Material Design Icons](https://github.com/google/material-design-icons),
  licensed under the Apache License 2.0: see [`LICENSE.material`](LICENSE.material).
  As tiny-dfr's README says, some of them are derivatives of Material icons with
  edits made by kekrby.
- tiny-dfr itself is MIT licensed ("Copyright (c) 2023 WhatAmISupposedToPutHere" in
  its LICENSE; "Copyright The Asahi Linux Contributors" in its README): see
  [`LICENSE.tiny-dfr`](LICENSE.tiny-dfr).

`install.sh` copies them to `/etc/touchbinux/icons/`, without overwriting files that
are already there. The battery widget looks for its `battery_*.svg` there.
