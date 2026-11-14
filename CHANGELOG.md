# Changelog

## [Unreleased]

### Added

- Plugins can be **hybrid** (`kind = "hybrid"` in `plugin.toml`): a native
  plugin with tools and routes whose window is its own framed web page, served
  like a Craft web app. This is what lets a game made with Godot run in a
  LiteAura window while its rules and the assistant's tools live in Rust.
- Oorbit 0.2.0, a slow planet game you can play: found a planet in one of
  four climates, then watch its people grow through the Stone, Agricultural
  and Bronze ages in real time. Choose what to study, build on its 32 tiles
  (each building has its own little model), decide where people work, and
  answer the events that come up. The Chronicle tells what happened while
  you were away. The planet is drawn as low-poly 3D in soft pixel art, under
  a starry indigo night or a lavender dusk, and the window works on a phone.
  In English and Italian.
- Oorbit 0.3.0: the first visit is lively. New settlers arrive with some
  knowledge and work up to nine times faster in their first hours, so the
  first discovery comes after ten minutes and the first buildings within the
  hour; the first three eras now take about two weeks instead of three. In
  the window, resources keep counting up, construction and research count
  down, the planet refreshes the moment something finishes, and little
  people walk around the inhabited tiles.

- Oorbit 0.4.0: planets have 72 tiles (12 pentagons and 60 hexagons) and
  look rounder; a planet saved with 32 tiles keeps everything it had and
  gains new land. Zoom in with the mouse wheel, a trackpad pinch or two
  fingers: up close the picture gets sharper, down to single pixels. The
  Society panel can start the planet over (two presses).

- Oorbit 0.5.0: all ten eras. After the Bronze Age come the Classical
  Age, the Middle Ages, the Renaissance, the Industrial and Electric Ages,
  the Neon Age (megatowers, neon strips, rogue AIs) and the Orbital Age,
  which ends with the orbital station. Each era brings its discoveries,
  buildings with their own little models, events and colours, and a full
  planet takes about two months. Old buildings can be demolished to make
  room.
- Oorbit's neighbours: three planets with their own personality (generous,
  trader, prankster) send gifts, ideas, visitors, pranks, trade offers and
  messages, and answer the ships you send from the dock. Ships cross the sky
  as what their era sails with, from seeds on the wind to rockets, and get
  faster and bigger with every era.
- Oorbit test tools (temporary): a fast-forward panel moves your planet's
  clock on by an hour, six hours, a day or three days.
- Oorbit 0.6.0: a new planet of 642 cells with continents, seas and beaches.
  Biomes fade into each other instead of meeting at a hard line, mountains
  are part of the ground, and trees, rocks and buildings stand on the land
  where they are (no more floating peaks). Zoom goes down to the surface,
  where the camera tilts to the horizon, and a double click flies over a
  cell. The village grows outward from its home: new buildings go next to
  the old ones. Planets from earlier versions are reborn on the new land and
  keep their era, discoveries, people, stock and buildings.
- Oorbit 0.6.1: an active game. Point at a forest, a hill, a meadow or the
  sea by the shore and a crew goes to work there; crews walk back to the
  village with their load ("+3 wood"), cells thin out while they are worked
  and grow back at rest, and buildings rise from their footing while
  builders work on them. The work sliders are gone: whoever has no crew
  thinks and makes culture. While you play, an hour of the planet lasts a
  minute; while you are away, the planet runs at half speed.
- Oorbit 0.7.2: sound and richer models. The planet has sound: the hum of
  space from afar, and coming down to the ground the sea, birds over the
  forests, voices over the towns and crickets on the meadows, mixed by what
  the camera looks at; effects for taps, building, hammers on the sites,
  discoveries, the new era, finds and boats fade with their distance from
  the middle of the view and are silent on the far side. A button on the
  left mutes it, and the window remembers. The sounds are synthesized by a
  script (`plugins/oorbit/scripts/synth-sounds.py`). Trees have trunks
  (conifers, broadleaves, palms by the sea), houses gable roofs, doors and
  lit windows, the first eras' buildings more detail (round huts, a fire
  pit with logs, trilithons, fenced fields and pastures with sheep, mine
  frames), people have heads, carts wheels, and boats a hull and a sail;
  fishing crews go out in a boat.
- Oorbit 0.7.1: more to do while playing. Things turn up on the land for a
  few hours (berries, lost goats, a crate washed ashore, a meteorite, wild
  horses, a wreck, old ruins): tap one and the people bring it home at
  once; a marker on the edge of the window points at the ones on the far
  side. Some events happen on a cell (lightning in a forest, a storm on the
  coast, traders on the plains) and the camera flies there. Help a building
  site and the thinkers finish it in half the time, at the cost of a pause
  in research. A new Goals panel always has three small goals with a
  reward. Sea routes are now a boat between two cities on the shore; the
  jetty is gone.
- Oorbit 0.7.0: the land runs out and the first session moves. Stone,
  clay and ores come out of deposits that end for good (a worked-out quarry
  shows as a pit), forests thin as they are cut and take days to grow back
  (plant trees to speed them up), and meadows and fish rest before they
  give again. Crews take at most three quarters of the workers, so someone
  always thinks; the window shows what one more crew costs in knowledge. A
  crew whose cell is built on moves to the nearest cell like it. Watching
  the planet now counts as playing, so it never slows down for idleness
  while you are there, and a first session reaches the Agricultural Age in
  about twenty minutes.
- Oorbit 0.6.2: cities. From the Agricultural Age a colony founds a new
  city on free land five cells or more from the others, and new buildings
  go up within four cells of a city. Link a colony to the capital with a
  road over land (laid cell by cell, carts travel on it) or, with a jetty in
  both cities, a sea route that boats sail. A colony with no way to the
  capital works at 60%; every linked colony adds 5% to knowledge and
  culture. Every city carries its name and its banner. Also fixed a dark
  streak across the sky, a villager drawn far off the planet.

### Changed

- Oorbit draws far less: one mesh for all the planet's props instead of
  hundreds of small ones, the palette pass at the low resolution only, at
  most 30 frames a second, almost nothing (and no refreshes) while its window
  is off screen.
- Web apps can ship precompressed files: the core sends `file.gz` or
  `file.br` to browsers that accept them. Oorbit's 40 MB game engine now
  downloads as about 10 MB.
- Oorbit's simulation (`crates/oorbit-sim`): the rules of the first three
  eras (Stone, Agricultural, Bronze) with people, mood, seasons, discoveries,
  buildings, monuments, events and named notable people. A planet left alone
  overnight catches up exactly as if the computer had stayed on, slows down
  when nobody plays, and never loses what it built. It is not in the window
  yet.

## [0.2.7] - 2026-11-12

### Fixed

- PhotoCraft 0.23.1 from the Craft store stayed on "Loading" and asked to
  reload: its web build imports a file from a folder where the release
  does not ship it. The store now fills such gaps when it installs an app,
  so the app starts.

## [0.2.6] - 2026-11-12

### Added

- Capture 0.1.5: the assistant can give a video a voice. It writes the lines
  and the local voices read them: one narrator, or a podcast or dialogue
  with two or three characters, each with their own voice (Supertonic, or
  Qwen3-TTS when installed). The lines land on the voice track and the
  assistant gets each line's time, so captions and graphics follow the
  voice. A video can now be made of voice and graphics alone.
- Capture graphics can use three.js for 3D (a solar system, a product
  turntable, a molecule): the library ships with the plugin, and WebGL
  renders in software when exporting, with no GPU needed.

### Changed

- The assistant's guide for Capture graphics no longer steers towards
  slide-like presentations: any form a browser can draw is open, from
  kinetic type and charts to canvas, shaders and full-screen 3D scenes.

### Fixed

- A window moved to another workspace was missing when the desktop switched
  there: the switch animation slid it out with the old workspace and hid it,
  until a click on its dock icon. It now slides in with its new workspace.
- The traveler map showed "API KEY REQUIRED" on every tile: CARTO basemaps
  now need a key. The map draws OpenStreetMap's own tiles, which need none;
  the dark theme is the same map with its lightness inverted. The server
  fetches the tiles (`/api/map/tiles/:z/:x/:y`) and keeps them in
  `~/.cache/shiny/map-tiles` for two weeks, as OSM's tile usage policy
  asks: loaded straight from the page, OSM blocked them ("Access blocked").
- Capture exports with a long or heavy graphic failed with "render timed
  out": the time limit now grows with the number of frames.
- The export progress stayed at 0 % while graphics rendered, which is most
  of the time for a 3D scene; it now moves with every frame.
- Exported graphics sometimes repeated a frame (a stutter), and single
  frames (thumbnails) could show the graphic at the wrong time.
- The assistant's voice and export requests worked only with the exact
  parameter names; the usual variations are accepted now.

## [0.2.5] - 2026-11-09

### Added

- Capture, a new plugin for screen recording, live streaming and video
  editing with the assistant:
  - Record the whole screen, a region (free, 16:9, 9:16 or 1:1) or one
    window, which is followed when it moves or is resized. A webcam bubble
    floats above every window; drag it anywhere, resize it, and shape it
    from square to circle, mirrored, with an accent border. The microphone
    and the desktop sound are kept on separate tracks, with level meters.
    Pause, markers, and a recording chip in the top bar that stays visible
    with the window closed. Notifications wait until the recording ends, so
    they never end up in a video.
  - Go live on YouTube, Twitch, Kick, Facebook or any RTMP server, several
    at once from one encode; a platform that drops reconnects on its own
    without stopping the others or the recording. Only the stream key is
    needed, and it is stored sealed.
  - Light on the machine: only the parts of the screen that changed are
    grabbed and converted (AVX2), unchanged frames are not encoded at all,
    and the GPU encodes. An idle 1080p recording costs about 1% of one core.
  - An editor for any video, sound or picture: a timeline with thumbnails
    and waveforms, split, trim, speed, fades, placement and rounded corners,
    captions, cutting by selecting lines of the transcript, export to
    1080p, 720p, 4K, vertical 9:16, square or GIF.
  - The assistant edits with the model chosen in Settings: proposals you
    accept, reject or refine, or "Edit everything" with a goal and
    guidelines. Its graphics are free HTML (titles, animated explainers,
    presentations) that keep one style from graphic to graphic until you ask
    for something different. Recording or going live is never started by
    the assistant without your confirmation.
- Shared UI: a timeline, time scales, a waveform and a level meter in
  `web/ui`, extracted from Studio; Studio's meters, waveforms and pinch zoom
  now use them.
- Shared crates `shiny-vault` (sealing, used by Browser and Capture) and
  `shiny-dsp` (loudness and waveform peaks, used by Studio and Capture).
- Whisper sidecar: `POST /stt/transcribe` returns word timings for a block of
  audio (Capture's editor transcribes recordings with it).
- Notifications have a quiet mode (`setQuiet` in `web/ui`): banners are
  held while something records the screen and shown afterwards.
- A plugin page served with a `sandbox` Content-Security-Policy keeps its own
  policy and can be framed by the app (an opaque origin with no access to the
  app); every other response still gets the app's policy.

### Fixed

- Capture 0.1.1: the preview is no longer empty before recording. It shows
  the screen, the chosen window or the region live, twice a second, so you
  see what will be recorded before pressing Record. The placeholder icon no
  longer covers the picture, and on a phone the preview keeps a 16:10 shape.
- Capture 0.1.2, tested end to end on a recorded window:
  - The editor's Export menu and clip menu, and the recording chip's menu in
    the top bar (Pause, Add marker, Stop), opened invisibly and could not be
    used. They now appear.
  - Undo in the editor did nothing after the first change, and pressing it
    again redid the change. It now goes back one change at a time.
  - Exports at 1080p, 720p and 4K cropped a recording whose shape is not
    16:9 (most windows), cutting its edges and the captions at the bottom.
    They now show the whole picture with bars; Vertical and Square still
    fill the frame.
  - The assistant's Capture guide now names every Capture tool (status,
    recordings, projects, import, transcription, proposals, export), so the
    skills check passes again.
- Capture 0.1.3, tested as a YouTuber would: a recorded tutorial with a
  Supertonic voice-over, transcribed by Whisper and edited by the assistant
  (DeepSeek):
  - Transcripts no longer repeat a word that falls on the edge between two
    30-second blocks ("trasporti" followed by "Sporti").
  - Cuts no longer leave an empty 40 ms clip that flashes a frame.
  - The assistant keeps its graphics clear of the captions and moves or
    fixes a graphic in place instead of adding a second copy.
- Capture 0.1.4: the preview stayed empty and nothing could be recorded on
  the installed system. The server runs as a systemd user service without
  `DISPLAY`, so Capture found no screen; it now finds the local X display
  itself (`/tmp/.X11-unix`), and the session script hands `DISPLAY` and
  `XAUTHORITY` to the user services before starting the server.
- Calc 0.1.2: typing on a selected cell kept only the second key onwards
  ("Voce" became "oce", "=SUM(...)" lost its "="). The first key now stays.
- The assistant no longer crashes its turn ("That one slipped") when a
  model's reply has a character like "€", an accented letter or an emoji
  right after a "<", as generated HTML often does.

## [0.2.4] - 2026-11-08

### Changed

- New defaults: the accent is black on the light theme (Dusk) and white on
  the dark one (Ink), with the matching monochrome gradient, instead of
  orange, and both themes open on the Lattice wallpaper. Colours anyone
  already picked stay as they are. The Dusk wallpaper is gone; a desktop
  that used it shows Lattice.

### Fixed

- Live session: changing the interface scale in Settings no longer fails with
  "Permission denied". The boot scripts that prepare the live account created
  `~/.config` and `~/.local` as root, so nothing in the account could write
  its settings there.
- T2 Macs: the Wi-Fi and Bluetooth firmware found in macOS is now installed.
  The renaming step used a command of get-apple-firmware that only exists on
  macOS, so the firmware was found but never put in place.

## [0.2.3] - 2026-11-08

### Changed

- T2 Macs: the live image and installed systems get the T2 kernel and the
  T2 userspace from the T2 repository, which installed T2 Macs keep, so
  `apt upgrade` brings new T2 kernels. Every machine set up by the
  installer boots the newest installed kernel, with no menu and no wait
  (`liteaura-update-boot`, run by the kernel hooks after each update).
- T2 Macs: Wi-Fi and Bluetooth firmware is read at boot from the Mac's own
  macOS (read-only), or from a copy on its EFI partition, and the installer
  carries it to the installed system, so erasing macOS keeps Wi-Fi working.
  The installer says whether it was found; without macOS, `get-apple-firmware`
  can fetch it later. Installer 0.1.2.

## [0.2.2] - 2026-11-08

### Fixed

- Voice: the small "Thinking…" bubble above the voice bar is gone. The
  voice bar's dots already draw every step, and the text chat shows the step
  in its own pending bubble. The empty pill a voice turn still left above the bar
  is gone too.
- Text chat: the thinking, chats and close buttons and the message field
  have an almost solid plate with a blur, so the conversation scrolling
  under them no longer shows through and blurs their labels.
- Appearance: the White accent picked on Ink no longer turns orange on Dusk.
  White cannot be read on a light canvas, so the light theme replaced it with
  its own orange default whatever colour had been chosen. A neutral accent
  now becomes the other theme's neutral (White on Ink, Ink on Dusk), and the
  stored choice comes back when you switch again.
- Browser 0.2.3: the start page's news no longer repeats itself or keeps you
  waiting. One article came back once per topic (the topics were searched
  separately and never compared) and the topics themselves were words of that
  same article's address, so the shelf was one story from every outlet. Now a
  story appears once (same address, or the same headline in other words), each
  publisher gets at most two cards, each page of the history counts once, and
  topics drawn from the same pages count as one; code hosting and AI-chat
  addresses are no longer topics. The shelf shows what arrived within 2.5 s and
  the rest fills in for the next visit, and news past ten minutes old is shown
  at once while it refreshes in the background.
- Plugins: an updated plugin now really runs its new code without a server
  restart. The reload reopened the library from the same path, and Linux
  handed back the copy already loaded, so the old code kept running while the
  server reported the new version.
- Plugins: a plugin's routes no longer wait for each other. Every request to a
  plugin ran one after another on its worker, so a slow one (a news search
  waiting on the network) held up the rest, down to saving a bookmark. They now
  run concurrently, still on the plugin's single thread.
- After login, the top-bar menus (power, battery, Wi-Fi), the browser's
  suggestions and the Craft apps took up to a minute to load. Chromium opens at
  most six connections to the shell's server, and each live feed held one for
  good: one per top-bar icon plus one per open Craft app, which filled them all,
  so every other request waited in line. All the live feeds now share a single
  connection (`GET /api/events`), leaving the rest free.
- Login is lighter. The speech engines (Supertonic, faster-whisper) still
  start with the server, in parallel with the desktop, but at a low CPU and
  I/O priority, so building the desktop gets the machine first. The session
  script no longer starts a second copy of them at full priority (which also
  ignored the AI switch). The session log now records when the desktop is shown
  and when voice is ready.
- Browser: a link or bookmark opened from a new tab's start page now opens
  in that tab, instead of in another new tab beside it.
- Browser: saving a bookmark now says whether it worked, and a failure is
  written to the session log with its reason.
- Browser: history was no longer saved and the home page had no news. The
  plugin wrote visits into its renamed history table, but the development
  install had copied the library without the migration that creates it. The
  plugin now creates the table itself when it is missing and moves the old
  history into it, and `scripts/install-plugin-dev.sh` copies migrations too.
- Browser: a page no longer goes blank while a menu is open over it (the
  Browser's own menus or a top-bar menu such as the battery one). The shell
  keeps the last masks that cut menus out of the page, and once it held more
  than 24 it evicted one at random, often the mask just made for the open menu.
  It now always drops the oldest.
- Browser: pages no longer get Chromium's automatic dark mode, which inverted
  their colours. They are told the system theme instead, Ink as dark and Dusk
  as light, through `prefers-color-scheme`: a site with its own dark style uses
  it and every other site looks as its author made it. Chromium reads this only
  when it starts, so after a theme switch pages follow from the next start of
  the shell.

### Changed

- Installer 0.1.1: the wording is plain and calm in every language, since
  the installer can erase a disk. Playful titles ("Make yourself at home.",
  "Ready for the one-way step.") now say what each step is, and leftover
  development text that made the real install look like a simulation is
  gone ("destructive engine not wired", "UI preview"). The install on the
  live ISO is real; a test mode exists only when the installer service is
  started with `--dry-run`, and it is called "Test mode". Typing the disk
  serial is no longer required: ticking the confirmation is enough, and the
  installer service still checks the disk's model, serial and size against
  a fresh probe before writing.
- Closing the text chat no longer stops the assistant. The answer carries
  on behind the closed panel with the voice bar's dots showing the work, and
  reopening the chat shows it where it stands, still streaming. Stop (or
  Escape while the chat is open) still abandons it. If the answer needs you
  to allow a command meanwhile, the chat stays closed: a notification offers
  to open it and the voice bar's ring breathes slowly until you answer.
- Windows no longer get an accent-coloured border when active. A coloured
  frame now means focus mode only (the palette frame of the eye button); the
  active window shows by its title and by stacking.
- Focus mode has a new entrance: a glowing comet laps the window from the
  bottom-left corner, the palette frame settles in behind it, and an accent
  bloom around the window settles into a soft glow that stays while the
  window is in focus.
- Focus mode and bring to front are no longer translated alike. In Italian
  the focus mode is now "focus" ("Metti il plugin in focus"), and every
  other language was checked: focus mode, bring to front and activating the
  next window each have their own wording. The English status of the AI
  raising a window now says "Bringing the window to front…".
- Kinetic motion no longer draws a line under the focused window's title;
  the focused title stays at full strength and the others dim.
- The Sound, Wi-Fi and Bluetooth menus now look like the Battery menu. Each
  opens with its state as one big value (the volume, the Wi-Fi signal, the
  connected device's battery) with the glyph and the switch at its right and the
  details in small capitals under it. The footers are gone: Scan and Test
  sound are icon buttons at the end of the section they act on, and the
  network in use is no longer listed twice (its row carries Disconnect and
  Forget).
- The top bar is now called the dock everywhere, since it can sit on any
  screen edge.
- The power menu (lock, sleep, restart, power off) is titled Session instead
  of Power, which read as electricity in Italian ("Alimentazione") and other
  languages.
- Browser 0.2.2: the address bar completes what you type. Typing `you` fills
  in `youtube.com` (Enter goes there, the next letter replaces it), and a short
  list under the bar offers matching bookmarks, visited pages and past
  searches, with arrow keys, Enter and Escape. New route
  `GET /api/browser/suggest`.
- The shell's file dialog (Open… in the web apps, the wallpaper and plugin
  upload pickers) is redrawn. Names no longer run into the next row: they wrap
  to two lines and end with an ellipsis, and the grid no longer depends on the
  Files window's styles. A path bar with a breadcrumb replaces the bare path,
  folders come first, videos show a still, the footer names the picked file,
  and the selection wears an accent ring with a check. On a phone the dialog
  is a bottom sheet with icon buttons; on the wearable face it fills the round
  screen with two columns and round actions instead of collapsing to a broken
  strip.
- Files quick look: the file keeps no frame or background but now has rounded
  corners (images, video, PDF pages, slides). The video player is new: a
  floating glass control pill with an accent play disc, a filled seek track, a
  volume level that slides out of the speaker, a full-screen toggle (also on
  double-click) and a big play button while paused, in place of the accent
  gradient bar over a black box.
- Debug builds are much smaller: the workspace keeps only file and line
  tables (backtraces still show `file:line`) and dependencies carry no debug
  info. `target/debug` had grown past 200 GB with full debug info on every
  copy. For a debugger session, build with `CARGO_PROFILE_DEV_DEBUG=true`.
  The getting-started guide also shows how to prune old build artifacts with
  `cargo-sweep`.

## [0.2.1] - 2026-11-07

### Added

- The README's tour video covers all three faces of LiteAura: the desktop
  (the assistant writing a document and a budget, the three window layouts,
  the overview, the themes), the phone and the wearable face. The recording
  scripts are in `scripts/readme-tour/`.

- A wearable face, for a watch but available on any screen (Settings →
  Desktop → Wearable): your wallpaper under the assistant's dots, now spread
  over the whole screen — semi-transparent, fading out in a circle, moving
  with the effect you chose — and a large clock with the date and the weather.
  No window ever opens: plugins work in the background and the assistant
  answers by voice, always listening for its name. One button changes the
  wallpaper, the AI model, the theme and the animation, or turns the face off.

- The voice bar is now a grid of dots that fills it edge to edge and moves
  with the assistant: with the voice while it listens and speaks, with the
  tools while it works. Settings → Appearance → "Voice bar effect" picks
  how: Glyph (the default — a dot-matrix display where the voice scrolls by
  and each tool draws its own little mark), Equaliser, Ripple or Fireflies,
  with a live preview beside the choice. "Dots react to" picks Voice, Tools
  or Both. "Voice bar style" keeps the top bar's plate by default (the dots
  in the theme's text) and gains Dark (light dots) and Light (dark dots),
  whatever the theme; "Voice bar colour" shows with the Accent style, for
  the accent or every accent at once.

- On a phone every window now reads like a phone app. A window's panes — a
  list and what it opens, a detail and its editor — come one at a time: the
  detail slides in over the list and a round back arrow, Escape, Android's
  Back or a swipe in from the left edge brings the list back. An action is
  an icon: buttons that carried words show their icon alone, round and
  finger-sized, and the few that have no icon are small; the OK and Cancel
  of every dialog are two round icons. The window's title bar is gone on a
  phone — the screen belongs to the app — and its Close and Focus buttons
  moved to the window switcher's cards, round on a plate, next to the
  app's icon. Back now closes what is open on top (a pane, a sheet, a
  dialog, the switcher) before it leaves the window. Windows may add a
  bottom row of icon actions for the thumb (compose, new, upload).
  All of it comes from the shared UI library (`paneStack`, `actionBar`,
  the back registry and the phone button policy), so every window and
  plugin gets the same behaviour; Plugins and Chats were the first to move
  onto it.

- Around the phone's apps: the home screen's icons wear a badge for what
  waits inside (unread mail, updates to install); a list pulled down past
  its top reloads (Plugins first); the window, the bottom bar and the chat's
  composer lift above the soft keyboard where the browser does not resize
  the page for it; and Settings → Appearance → "Window titles on a phone"
  puts the title bars back for whoever wants them.

- Calendar on a phone: the month fills the window and a day opens as its own
  pane over it, with Back returning to the month; wide windows keep the month
  and the day's events in one column. Today, New event, Edit, Cancel and Save
  are icons, and a bottom action bar puts New event under the thumb. The old
  rule that squeezed the day's events to a third of the window is gone.

- Files reads like a phone app: the toolbar is the window bar, the places
  become a strip above the folder when the window is narrow, and the bottom
  row carries New folder and Upload. One tap opens a file on a touchscreen, a
  long press opens its menu, and Select lets you tick several files before
  acting on them. The share dialogs' buttons are icons on a phone, and the
  quick look's arrows no longer need a mouse.

- Updates on a phone: every action is its icon, with the number of pending
  updates as a badge on the install button, and the package versions sit
  under each name instead of being squeezed beside it. Details opens the
  update log as its own page with a Back button; in a wide window the log is
  a column beside the list, and the same button hides it again.

- Settings reads like a phone app: the list of sections is the first
  screen, a section slides in over it with a Back arrow and its name in a
  slim bar, and the "At a glance" summary opens from an info button there.
  Every action in Settings is an icon on a phone; the profile name and photo
  now save as soon as they change, so the Save button is gone. The layout
  switch (Tiling, Columns, Windows) shows icons.

- Mail on a phone works one pane at a time: the folders sit in a strip
  across the top, the message list is the floor and an open message slides
  in over it, closing with Back, Escape or a swipe from the edge. Every
  button in the window, the composer and the account manager is an icon on
  a phone, and a Compose button sits along the bottom within thumb's reach.

- YouTube reads like a phone app: the search pill, Home and Refresh share
  one row with the window controls, and on a phone or in a narrow window
  playing a video slides the player in over the grid, with Back, Escape or
  a swipe from the edge returning to the results. The "Up next" row follows
  the player there, and the old text Back pill is now an icon.

- Word reads like a phone app. On a phone the formatting tools live in a
  bottom sheet behind one button in the bar, so the window keeps a single
  toolbar row; a tap on a tool applies it to what you had selected and tucks
  the sheet away. The outline slides in over the page from a list button in
  narrow windows and on phones, with Back to return, and picking a heading
  jumps to it. Every action in the bar and the Design dialog is an icon on a
  phone.

- Installer: on a phone or in a narrow window the installation summary is a
  pane that slides over the current step from an info button in the footer
  and closes with Back or Escape; the step rail becomes a strip above the
  step. The footer's Back, Next, Re-check, Install and Cancel are icons on a
  phone. Dragging a partition edge with a finger shows its size while pressed
  and for a moment after letting go; the remove button no longer pokes out
  of a sliver-thin partition.

- Calculator: the history is now a pane that slides in over the keypad in a
  narrow window or on a phone, with a Back arrow and the clear action in its
  head; a wide window keeps it under the keypad, or beside it when there is
  room. Tapping an entry brings its result back and returns to the keypad.
  On a phone the keys grow to fill the window like a phone calculator.

- On a phone or in a narrow window the PDF window reads like a phone app:
  the page fills the window with the thumbnails in a strip above it, and the
  comments slide in over the page from the Comments button in the bar, with
  Back, Escape or an edge swipe closing them. Every action in the bar, the
  mode strip, the selection bar, the comment cards and the dialogs is an
  icon on a phone. On a touch screen each thumbnail keeps its page-actions
  button, and the page menu offers Move earlier / Move later so pages can be
  reordered without dragging.

- Impress reads like a phone app: the slide strip becomes a row of
  thumbnails above the stage, and the slide settings (layout, transition,
  bullets, notes, slide actions) slide in as a pane from a properties
  button in the window bar, closing with Back or Escape. New, Present and
  Add slide are icons, and a phone gets an Add slide button at the thumb.
  Wide windows keep the inspector under the stage.

- Browser on a phone: one header row — the address bar is the window bar,
  with back, reload, the open-tab count and the menu — and the tab switcher,
  history, bookmarks, downloads, passwords and import open as full screens
  over the page with a Back button, closed by Escape or the phone's Back.
  Every panel action is an icon with its label, the icon alone on a phone;
  the start page's Refresh draws the theme's glyph. Desktop windows keep
  their tab strip and popovers.

- Calc reads like a phone app: the New action and the empty state's New
  spreadsheet / Import CSV / Import .ods are icons (the empty state names
  them in captions underneath), and the grid can be zoomed — pinch with two
  fingers, Ctrl+scroll, or the bar's zoom tools. On a touchscreen a second
  tap on the selected cell edits it in place and a long press opens the
  cell's menu with Edit cell and Clear cell. The in-cell editor, which never
  appeared, shows again.

- On a phone the Image editor gives the picture the whole screen: looks and
  adjustments open as a bottom sheet from two icons in the window bar, with
  a switch between them. Every one-click action has an icon, and the canvas
  now understands a two-finger pinch and a double-tap to zoom.

- Studio reads like a phone app: in a narrow window or on a phone the
  arranger is the floor and the detail panel, the browser and the AI panel
  slide in over it one at a time, with Back, Escape and the edge swipe
  closing them. The editor's actions are icons on a phone, the hint row is
  gone there, and a long press on a clip, a track or a note opens the same
  menu as a right-click — tapping a clip opens its editor.

- The machine's login screen wears the look each account picked in Settings
  — theme, accent, gradient and motion style — instead of the default light
  theme and orange accent. Each session publishes its account's look when it
  starts and whenever it changes, to `/var/lib/shiny/looks` (the greeter is
  another account and cannot read Settings); the login screen opens in the
  look of the account used last and changes with the account pointed at.
  Only colours, a theme name and a motion style are published, and the
  greeter takes a file only from the account it names. The login screen's
  launcher starts the renamed `liteaura` shell too.

- Import from another browser, in the Browser's main menu. History and
  bookmarks come in one click from the browsers installed on the computer —
  Firefox, LibreWolf, Chrome, Chromium, Brave, Edge, Vivaldi, Opera, also as
  Snap or Flatpak — read from a copy of the profile, so a running Firefox is
  no obstacle. Passwords come from the CSV a browser or password manager
  exports (Firefox, Chrome, Edge, Brave, Safari, Bitwarden, KeePassXC,
  1Password, recognised by the header), bookmarks also from the HTML every
  browser exports. Importing again adds nothing twice; a login already saved
  is updated, not duplicated.

- The Browser's password manager works on its own, like a browser's: after
  a sign-in it offers to save the login ("Save", "Not now", "Never" for the
  site), or to update it when the password changed; on a login page it fills
  the site's saved login into empty fields (with the vault locked it offers
  to unlock, once per site); on a sign-up form it offers a strong generated
  password (20 characters, every class, no look-alike characters) and puts
  it in the password and its confirmation. "Save" with the vault locked
  opens the unlock prompt and then the editor, prefilled. Incognito tabs
  fill but never save. The editor has the same strong-password suggestion.

### Fixed

- Craft apps get live AI control. The assistant's tools for an open Craft
  app now act on the window you see — the document appears and changes in
  front of you — instead of always working on a hidden copy. The page looked
  for the app's control API once, when the frame loaded, but a Craft app
  defines it only after its WebAssembly has started (VectorCraft: about 3.7 s
  later), so live control never attached. The window now keeps looking for up
  to two minutes, and the next turn switches the app's server to the live
  channel (`web/js/webAppSurface.js`).

- In the Columns layout a long document no longer stretches its window
  taller than the screen, and in the Windows layout new windows open in a
  cascade instead of exactly on top of each other.
- Plugins open the same database as the server when its address carries
  options (`sqlite://…/shiny.db?mode=rwc`): they used to create an empty file
  named after the whole address, so Word, Calc and the rest found no tables.
- The wearable face's settings are a panel made for a round screen: round
  wallpaper swatches and rows that step to the next AI model, theme or
  animation on a tap, all inside the circle; a 12-hour clock keeps its AM/PM
  small so the time fits.

- The assistant no longer loses a plugin's tools halfway through a request.
  After its first step (a web search before writing the document) it worked
  without the tools' docs, so "write it in Word" became a string of made-up
  tool names until it went looking for Word's skill in the terminal. Every
  step now names each plugin's tools, and a plugin's full skill comes back as
  soon as the assistant activates it, opens its window or guesses one of its
  tool names wrong. A new test checks that every plugin's skill names all of
  its tools and only real ones.

- Every plugin's instructions for the assistant were checked against what
  its tools really do, and corrected where they differed: missing tools
  (Studio's presets, Files' whole tool list), parameters that do not exist
  (Word's margins, Files' "empty trash"), missing ones (Radio's genre, country
  and language, Calc's sheet size, Image's colourise and gamma) and the
  answers each tool gives back. A few tools now also do what their
  instructions promised: Image reads and edits the latest picture when none
  is named, Mail takes a single address as a plain string, and YouTube
  suggestions work from a video id alone. Two PDF shortcuts no longer
  collide with Word's and with each other (`replace_text`, `add_note`).

- Fixes found while checking the assistant's instructions against the tools:
  a PDF watermark goes only on the pages asked for; a spreadsheet created
  with a wrong cell name says so instead of coming out empty, and a sheet
  can be deleted by its title; rewriting a presentation with no slides is
  refused instead of emptying it; Word's document design takes margins
  again; MIDI effects after "humanize" in Studio play; a file the assistant
  put in the Trash can be restored with the name it was given; all-day
  events keep no stray times; an email with neither subject nor text is not
  sent, and a search without a folder also looks in folders never opened;
  on a vertical screen the assistant no longer claims to have switched or
  moved desktops; routes only take car, bike or foot; trip statistics show
  the real duration and the first and last position; and an update check
  served from the cache is no longer flagged as out of date.

- A right-click inside a Browser page opens the menu again (Back, Copy,
  Paste, "Fill password…" on a field): the shell was dropping the request
  before it reached the window. A site with several saved logins — common
  after importing passwords from another browser — no longer stays silent:
  the Browser says so once and "Fill password…" lets you pick the account.

- Browser pages no longer stay on screen after the app reloads (an update
  without restart, a crash recovery): a page left behind did not follow its
  window and did not close with it, because the new document knew nothing of
  the native views the old one opened. The shell now closes them when the app
  starts loading, and the app closes any it finds recorded from its previous
  load.
- A machine set up under the former name keeps its site logins: the session
  script moves the shell profile from `~/.local/share/shiny/peakd` to
  `~/.local/share/shiny/liteaura` before the shell starts (the shell's own
  migration only covered its XDG folders), once and never over an existing
  profile. A development machine still launching the `peakd` shell from an
  old `/usr/local/bin/shiny-session` showed blank pages in the Browser once
  its web code spoke the renamed protocol; re-run
  `scripts/install-linux-session.sh` (or install the rendered script) after
  the rename.
- A plugin directory with a leftover library next to the rebuilt one loaded
  the leftover: the loader only recognised `lib<name>.so`, never the
  `libshiny_<name>_plugin.so` every build produces, and fell back to the
  alphabetically first file. A development machine kept running an October
  Browser library this way. The built name now wins.

### Changed

- The live ISO installs LiteAura from its signed APT repository while it is
  built, with apt inside the image, instead of packing files from the working
  tree: the image carries exactly the published version with its dependencies
  resolved, and the live account is seeded by that same server. The live
  session still signs in on its own; the installed system uses LiteAura's
  login screen.

- The dots replace the voice bar's other animations: the glow that swelled
  with the audio and the "Tool status" setting (the state word, the step
  line or the tool's icon inside the bar) are gone. The step line stays in
  the bubble above the bar.

- The Browser's tabs look like the Terminal's — flat mono capitals, the
  active tab a pill, a tab's close on hover — because both windows now use
  one shared tab strip from the UI library (`tabStrip` and `tab` in
  `web/ui`), with the touch sizes in the shared responsive rules.

## [0.2.0] - 2026-11-06

### Fixed

- The assistant no longer runs the same action over and over. A model that
  did not register a result asked for one identical `calc_create` forty times
  in a turn and left forty spreadsheets; an exact repeat of the call that just
  succeeded is now not run again, the model is told it is done, and after
  three repeats the turn ends.
- "Show me Word" opens Word. `show_plugin` refused a plugin that was installed
  but not active this session (every plugin, after a sign-in without
  "Remember my workspace") and the model then answered "it is open" over the
  error; it now activates the plugin and the window opens.
- The assistant knows today's date: without it "November 20" became an event
  in 2024.
- When a plugin's tool succeeds but its window is closed, the model is told
  so, with the call that opens it, instead of claiming the result is on screen.
- The docs no longer suggest copying a rebuilt plugin library over the
  installed one: rewriting a loaded `.so` in place crashes the server
  (reproduced, exit 139 at shutdown). Libraries are replaced by writing a new
  file and renaming it, as `scripts/install-plugin-dev.sh` does.
- Calendar events accept the shapes models send: `summary`, `name` or
  `subject` for the title, and `start`/`end` as ISO date-times
  (`2026-11-20T08:15:00`); times with seconds are accepted too. Calc answers
  to `create_sheet`.
- Tool calls written as markup instead of JSON run instead of being printed in
  the chat. DeepSeek, over an OpenAI-compatible API, writes its calls as
  "DSML" tags (`<｜DSML｜invoke name="…">` with `<｜DSML｜parameter>`), other
  models as `<invoke>`/`<parameter>` or Qwen/Hermes
  `<tool_call>{"name", "arguments"}</tool_call>`: the call was shown as raw
  markup and the turn ended without doing anything. These forms are parsed
  like the JSON blocks, so the repeat guard covers them too, and any markup
  left in a reply, even malformed, is hidden.

### Changed

- README: LiteAura throughout (the core keeps its internal name `shiny`), the
  features as they are now — phone layout, lock screen, its own login screen,
  updates without a restart, the assistant Laura, 26 languages — and
  a tour of the assistant at work — asked in the chat, it writes a document,
  adds a calendar event and builds a spreadsheet, each in its window — then
  the layouts and the Ink theme (animated WebP, with an MP4), and screenshots
  of the tiling and free-window layouts and of the phone, in the Dusk and
  Ink themes (`docs/screenshots/`).

### Removed

- The transitional `peakd` package and the `peakd` APT suite. They existed
  for systems installed under the former name, and there are none: the
  repository now publishes only the `liteaura` suite, with `liteaura` and
  `liteaura-archive-keyring`, and `liteaura` no longer declares
  `Replaces`/`Breaks: peakd`. The shell still moves a development machine's
  `peakd` folders, variables and browser history on its first start.

## [0.1.9] - 2026-11-05

### Fixed

- The package depends on the QML modules the shell loads (QtQuick, its
  window and effects, QtWebEngine, QtWebChannel). Without them a clean
  install opened a black screen with a cursor: the shell exited at once with
  `module "QtWebEngine" … not found` and the login screen kept restarting it.
- An account's first session goes straight to the desktop. The per-user
  server now creates the account of the Linux user it runs as on its first
  start, so the kiosk's boot token has an account to sign into; before, a new
  account was asked for its password a second time by the app after the
  login screen.
- The APT repository no longer goes offline between releases. GitHub Pages
  was set to deploy from the `main` branch, so every push replaced the site
  (and `apt/` with it) until the next tag; it now deploys only from the
  release workflow. The requirement is in `docs/deployment/packages.md`.

## [0.1.8] - 2026-11-05

### Added

- The Debian package carries LiteAura's own login screen: the greeter, the
  PAM helper (`shiny-auth`), the per-account session and server, at system
  paths (`packaging/seat`). It is off after `apt install`, so a desktop keeps
  its login; `sudo liteaura-seat enable` puts it on the seat (disabling the
  display manager), and the installer does that on a new system instead of
  setting up LightDM autologin. The AI tier chosen in the installer is now
  the machine default in `/etc/default/liteaura`. LightDM is no longer
  recommended by the package. See `docs/deployment/packages.md`.

## [0.1.7] - 2026-11-05

### Added

- Updates take effect without a restart. The server fingerprints every
  plugin directory and loads again the ones whose code changed on disk —
  after a package upgrade, a Craft store install or a copied build — on
  `POST /api/plugins/rescan` and on its own every ten seconds
  (`src/plugins/digest.rs`, `application::plugin_watch`). When its own package
  was upgraded, the server swaps itself for the new binary in the same
  process, handing the listening socket over, once the assistant is idle
  (`POST /api/system/reexec`, `application::reexec`): the port never closes.
  The page follows (`web/js/liveUpdate.js`): it reads `/api/system/version`
  every twenty seconds and after any lost connection, reloads a changed
  plugin's window module (in place when the window can carry its state,
  `saveState()`/`restoreState()`), and reloads itself once when the core web
  tree changed, restoring every window — also with "Remember my workspace"
  off. The Updates window does all this after a successful upgrade and says
  what happened. The kiosk shell, the one piece that needs a relaunch, does it
  behind the next locked screen (exit code 44). See
  `docs/core/live-update.md`.

## [0.1.6] - 2026-11-05

### Added

- Lock screen, on the desktop and on a phone: the time set large, the date and
  the weather, in the theme, the accent and the motion language. It locks after
  an idle time, before a suspend, from the power menu's **Lock** and on
  `Super+L` / `Ctrl+Alt+L`; the desktop behind it is hidden (inert, invisible,
  the Browser's native page sent away). A phone swipes up to unlock, a desktop
  moves the mouse or presses a key; the account password is asked only when
  Settings → Account → Lock screen says so, checked without opening a new
  session (`/api/auth/verify-password`). The lock screen itself and the idle
  time — any amount of seconds or minutes, or never — are set there too
  (`web/js/lockScreen.js`).
- Craft store: new Craft apps join the store on their own. Once a day it
  reads the ArtCraft launcher's release feed and the `storytold` GitHub
  account, and adds every app whose release carries a web build; what it
  finds is remembered across restarts (`src/plugins/craft_discovery.rs`,
  `<plugins_dir>/.craft-store.json`). TextureCraft, MaterialCraft, Craft
  Anything and SolidCraft (community, Tangency Labs) are in the built-in
  catalog too.
- Craft store: each app's brand colour, publisher and community flag in the
  catalog, and its icon before it is installed
  (`GET /api/plugins/store/icon`).

### Changed

- The assistant's default name is **Laura**: "Hey, Laura" wakes it on a new
  account (`web/js/preferences.js`). A name chosen in Settings → Assistant
  stays as it is.
- The shell is now called **LiteAura** (the name PEAK'D! was taken): the
  crates (`crates/liteaura`, `liteaura-core`, `liteaura-mac`), the binary, the
  `LITEAURA_*` variables, the live-session scripts, the Debian packages and
  the APT suite, and every translation of the app name. An existing install
  keeps everything: `~/.config`, `~/.cache` and `~/.local/share` `peakd`
  folders and the web profile (cookies, site logins) move on the first start
  (`liteaura_core::legacy`), `PEAKD_*` variables still apply when the new name
  is unset, the Browser copies `peakd_history` into `liteaura_history`, and a
  transitional `peakd` package plus a `peakd` APT suite keep installed systems
  updating and starting.
- Words no longer break letter by letter in narrow places: a segmented
  control's segment is never narrower than its longest word (the row wraps
  between segments instead), labels and values use `overflow-wrap:
  break-word` rather than `anywhere`, the Settings summary's values keep
  their words, Mail's reader shows its actions as icons in a narrow column,
  and long radio station names hyphenate.
- Translations: every interface language now has every string of the shell
  and of the Files, Studio, Mail, PDF and Terminal windows.
- Sign-in screen: an editorial stage instead of the card — the time set very
  large with an accent colon, the date and the weather beside a numbered profile
  index in display type with arrow pills, and the password on a hairline with a
  round accent "go". It opens in the look of the last profile signed in on this
  device (accent, gradient, motion) and takes the look of the profile pointed
  at; it arrives, changes step, refuses a wrong password and leaves over the
  desktop in the chosen motion language (kinetic masks, fluid springs, depth
  blur and parallax). The lock screen is the same stage (`web/js/gate.js`,
  `web/css/gate.css`).
- Plugins window: rebuilt in the editorial language of Settings and the
  Installer — a numbered rail (Craft apps, Installed, Updates, Install
  activity), an index of apps in display type with arrow pills and the
  download filling each row's hairline, and a live manifest with the selected
  app as a 3D slab in its brand colour that tilts with the pointer and idles
  in the chosen motion language. Narrow windows and phones get the manifest as
  a sliding panel and the rail as a strip.

- Window overview: the windows are a grid of cards with a small title and a
  close button under each, instead of the strip or the cloud — no headline,
  no Prev / Next, no drift. The grid scrolls when the windows outnumber the
  screen (wheel or drag), and the cards travel in the motion language picked
  in Settings → Appearance → Motion: kinetic in a fast cascade, fluid on a
  spring, depth out of a blur (`web/js/overviewGrid.js`). Themes no longer
  pick an overview layout (`overview` in `theme.json` is gone).
- Phone: the top bar is flush with the top edge (no rounding, no gap under
  it) and carries no app icons; the control centre unfolds from it edge to
  edge.
- Phone: one bar at the bottom. The thin strip with its line is also the
  assistant, and the voice bar no longer shows on a phone: a tap goes home
  (or stops a listen), a press and hold opens the switcher, a swipe up opens
  the chat, a swipe up and hold listens. Dragging sideways moves the window
  with the finger while the next one comes in beside it, like the workspace
  carousel on a desktop; let go past half-way, or flick, to switch.
  The line takes the assistant's colour while it is busy.
- Phone: windows fill the screen from the status bar to the bottom bar.
- Browser on a phone or in a narrow window: the toolbar is one row (back,
  reload, the address, the menu) instead of the address over a scrolling row
  of buttons; forward, home, bookmark, passwords, ad blocking, downloads and
  incognito sit as a row of icons at the top of the browser menu. Its menus
  and panels open under the toolbar instead of over it.
- PDF on a phone or in a narrow window: the window bar folds into the shared
  More menu like the other windows (it overflowed, because its tools sit in
  clusters within clusters), and the Write view's formatting row folds the
  same way instead of wrapping onto several lines. `compactToolbar` reaches
  into boxes marked `data-fold`, folds plain buttons marked `data-foldable`,
  and takes a `fold` selector for rows of non-library buttons.
- Phone check of every window bar: the document title in a compact bar now
  gives way down to 40px, and controls that never folded can now fold
  whole into More (`data-foldable`) — the view switches of Word, PDF and
  Studio, Impress's theme, Word's font and size, Studio's tempo. Studio keeps
  Play on the bar and drops its clock, beat and save dot while compact. With
  a message open, Mail keeps Reply and Close on the bar and folds Forward and
  Delete. Updates' bar is tabs, so it scrolls like the other tab strips.

## [0.1.5] - 2026-11-03

### Added

- Phone shell: on a phone LiteAura reads like a mobile OS. The top bar is a
  status bar (time, weather, the apps, a control button); the host status chips
  and the bar actions live in a control centre sheet (`web/js/controlCenter.js`).
  Windows fill the screen one at a time; with none on screen the launcher grid
  is the home screen. The dock around the voice bar has no buttons: tap its
  handle for home, hold it for the window switcher; swipe up for home, swipe up
  and hold for the switcher, swipe sideways for the next/previous window; the
  browser's Back goes home (`web/js/phoneShell.js`, `web/css/mobile.css`).
- Phone preferences are a view: the bar on top, rail style, no scale or
  autohide, the voice bar alone at the bottom, the grid launcher
  (`effectiveImmersive()`, `effectiveLauncher()`); stored desktop choices are
  untouched and the matching Settings rows rest on a phone.
- Compact toolbars (`compactToolbar`, `web/ui/compact.js`): in a window up to
  700px or on a phone every window bar shows icon-only actions and folds what
  does not fit into one shared "More" menu — same elements, nothing lost, no
  sideways scrolling. Word's formatting row uses it too; Mail puts the open
  message's actions on the window bar.
- Terminal: a key row on phones — Esc, Tab, a sticky Ctrl, arrows, `- / | ~`.
- PWA manifest and `viewport-fit=cover` (safe areas on notched phones).
- Top bar size: Settings → Appearance → Top bar size scales the whole bar
  (80–160%) — pill, icons, type and spacing — with everything that docks to it
  following its footprint (`--hud-bar-scale`, `--hud-bar-extent`).

- Files: public share links. **Share link…** (right-click a file or folder)
  uploads it to a bundled Gokapi server with expiry, download limit and an
  optional password; **Shared links** lists, copies and revokes them. Settings →
  Remote → File sharing turns it on and publishes the links through Tailscale
  Funnel or a custom URL. Only Gokapi's download pages are public, through a
  share gate that answers 404 to everything else (`plugins/files/src/share/`).
- Packaging: the `.deb` bundles the unmodified Gokapi release binaries
  (pinned SHA-256s, `packaging/gokapi.sh`), with its license and NOTICE; the
  release publishes Gokapi's Corresponding Source next to the package
  (AGPL-3.0 §6(d)).
- LiteAura: browsed pages follow the interface scale, like the app view around
  them (Linux and macOS shells).

### Changed

- Remote settings: the Tailscale hints give the exact command to run
  (`sudo tailscale up --operator=$USER`).
- Word reflows the document to a phone-narrow window at 100% instead of
  shrinking an A4 page to 40%; rulers and page guides step aside there.
- Mail shows one pane at a time in a narrow window: the list, or the message.
- Files: the phone header keeps navigation on one row and the search under it.

### Fixed

- Settings and Plugins opened on a phone now come to the screen instead of
  staying behind the window already shown.
- Files: the grid/list icon no longer fills its whole button.

## [0.1.4] - 2026-11-02

### Added

- Launcher: the plugin launcher (three-finger swipe down) is rebuilt with four
  layouts — grid (every plugin as a big icon), sheet (a sheet from the top
  bar, plugins by category), command (a keyboard-first palette) and mosaic
  (recent files as tiles above every plugin). It searches plugins and files,
  and can show the running windows; style, recents and previews live under
  Settings → Appearance → Launcher (`web/js/launcher.js`,
  `css/launcher.css`).
- Files: a Recent place — the most recently modified files of the classic home
  folders (Desktop, Documents, …), hidden entries skipped and the walk bounded
  so a huge Downloads folder cannot stall it; the launcher's mosaic reads the
  same list (`files::fs_util::recent`).
- LiteAura gestures: two-finger pinch events (scale per step, recognised against
  a two-finger scroll) reach the web layer beside swipes
  (`crates/liteaura/src/gestures.rs`).
- "No slop": the AI can be switched off entirely
  (`src/services/ai_switch.rs`). Per user the `ai.enabled` preference, per
  machine a small state file the session script reads before the server
  starts; with it off chat and agent requests are refused, no speech sidecar
  starts and Ollama stays down. The voice bar opens the launcher instead, and
  a context menu offers "Turn on the AI…"; switching it back on in Settings →
  Assistant starts Ollama and lets the watchdog bring the voice sidecars up —
  no restart.
- `liteaura-ollama-start` starts the local Ollama server, and both callers use
  it: the login session (`liteaura-live-session`) and the server when the AI is
  switched back on.
- Installer: the AI tier is chosen at install — off ("No slop"), none ("Text
  only"), voice ("Voice pack"), local ("Local AI") — and written as
  `LITEAURA_AI_DEFAULT`; a fresh account inherits it. The disk timeline
  describes partitions as a person would ("Windows stays untouched" rather
  than "ntfs · /dev/nvme0n1p3") and names what is erased or kept at every
  outcome.
- Mail: attachments. Outgoing mail takes files by home-relative path (the
  assistant's `mail_send`) or as uploaded multipart fields (the compose's
  attach buttons); received attachments are read back and saved into the home
  (`mail_attachment`, `mail_attachment_save`); either path shares the same
  18 MB ceiling (`plugins/mail/src/attach.rs`).
- Plugin SDK: `read_upload_file` parses multipart bodies directly (multer) —
  axum's Multipart caps them at 2 MB. Calc, Impress, Word, PDF and Mail import
  through it; Calc's import ceiling grows to 64 MB.
- Settings → Appearance: schematic previews of the style, windows, bar and
  launcher, drawn only with the theme's own tokens so they always match, and
  the section is paginated — Style, Windows, Bar, Launcher, Background
  (`web/js/settings/previews.js`, `css/settings-previews.css`).
- Chat: identical repeated tool calls fold into one card, counted ("Done N
  times"), kept where the first happened, instead of a stack of copies.
- Studio: the arranger zooms (4–240 px per beat, the view grows as the user
  scrolls right), a pattern clip can start offset into the beat
  (`offset_beats`), the editor preview crossfades when the pattern changes,
  and renders are coalesced — at most one render in flight with newer
  requests taking over, plus a process-wide lock so a long project cannot
  render several mixes at once against a ~3 GB buffer budget.
- Dev helpers: `scripts/install-plugin-dev.sh` replaces a locally built plugin
  atomically (library, manifest, skills, web) in both plugin roots with an
  optional service restart, and `scripts/install-studio-dev.sh` does Studio
  the same way.

### Changed

- Browsed pages follow the app's colour scheme: the shell binds WebEngine's
  dark mode to it, so pages report `prefers-color-scheme: dark`, a site with
  its own dark style switches to that, and only a page without one is
  darkened by the engine.
- The interface-language hint reflects that the voice follows the interface
  language when a speech engine is available for it.

## [0.1.3] - 2026-11-01

### Added

- Word: the document engine is rebuilt on WordCraft's libraries (see the
  plugin's `NOTICE.md`). Import accepts `.docx`, `.rtf`, `.md`, `.html` and
  `.txt` beside `.odt`; export writes polished `.docx` or PDF (WordCraft's
  patched krilla, added to the workspace as `krilla`). Tables, alignment,
  colours, sizes, lists and fonts survive every round trip.
- Word: a Design (Plain, Classic, Modern, Minimal templates) is stored beside
  the content (`migrations/002_design.sql`); the window paginates against it
  (page previews through `/api/documents/:id/pages`, fonts served at
  `/api/word-fonts`), and a new AI menu asks in the chat with the selection
  quoted, so the agent edits the document the window has marked as opened
  (`POST /api/documents/:id/opened`).
- Studio: a real mixer — per-track inserts, reverb/delay sends with returns,
  a master chain (bus, glue compression, look-ahead brickwall limiter,
  loudness trim) and cached stems per track, so the window plays them through
  its own faders for live monitoring that matches the export.
- Studio: audio tracks — sandboxed WAV imports with sinc resampling, fades,
  looping and tempo detection, a shared audio library, and new tools
  (`studio_import_audio`, `studio_arrangement_edit`, `studio_mix_analyze`,
  `studio_export`, `studio_audio_info`).
- Studio: the window can render AI songs in the background —
  `POST /api/songgen/generate` starts a job, `GET /api/songgen/jobs/:id`
  reports it, and finished tracks are placed on audio tracks; missing models
  are reported (with sizes) instead of being downloaded implicitly.
- Updates: structured progress for package operations. apt's
  `APT::Status-Fd` turns into per-package states (waiting, unpacking,
  configuring, installed, removed) that the window shows as a progress bar
  and package rows, polled alongside the log; a reopened window re-attaches
  to an install still running, and package lists can be refreshed.
- Browser: the news shelf also follows favourite sites — hosts visited often
  (same decay as the interest profile) get their RSS/Atom feeds discovered
  once from their home pages, and their items are interleaved like interests;
  the browser menu can turn news off entirely.
- Browser: a password vault — entries are Argon2id-wrapped records unlocked
  with the login password (via `/api/auth/verify-password`) and filled only
  on origin matches.
- Terminal: four window styles (Settings → Appearance → Terminal) — Minimal
  with a smear cursor, Glass with an accent beam and sparks, Paper with a
  highlighter trail, Blocks with spring cursor and command cards — drawn by
  `web/fx.js` while something moves; shell hooks (OSC 133/7) feed command
  markers, exit codes and the cwd.
- Editorial themes: Dusk and Ink share a new editorial design language that
  replaces the retired themes, and stored theme choices migrate forward.
- Appearance: four top-bar styles — rail (the classic pill), isles (time,
  apps and status as three pills), strip (an edge-to-edge line of type) and
  capsule (compact, opens on approach) — drawn by `css/barStyles.css`; every
  style docks to every position and the voice bar moves with the bar.
- Voice: the stop phrase that ends a hands-free conversation is understood in
  every interface language, matched whole-utterance and forgiven of case,
  accents and punctuation in any script; each catalogue quotes its own phrase
  and `web/js/tests/voiceStopPhrases.test.mjs` keeps the two in sync.
- `askInChat` (`web/js/textInput.js`): a plugin window can ask in the chat,
  and the request, its tool cards and the answer appear exactly like a typed
  turn.
- Files: `.docx` and `.rtf` open with Word, and a file dragged out of the
  window carries a typed payload (`application/x-shiny-file`) with its
  home-relative path.

### Changed

- Studio's DSP is written from scratch, block by block: mip-mapped additive
  wavetables that cannot alias, truly stereo ZDF filters, click-free
  transitions, loudness reached before the limiter and dithered 16-bit
  export. `fundsp` is gone — and with it the dev-profile workaround for its
  opt-level 0 speed; a `bench` example renders the whole chain for timing.
- The desktop greeting is gone (`deskGreeting.js` and `#desk-greeting`).
- User-facing wording speaks of "the assistant" rather than "the agent".

### Fixed

- Browser: editing settings keeps unknown columns (`news` is added to tables
  created before it existed) so a core/browser mismatch cannot lose
  preferences.
- Agent text: route planning and step wording say what actually happened
  (e.g. "No route found", "Going full screen…").

## [0.1.2] - 2026-10-28

### Added

- Browser: the start page is now a bookmark grid with each site's own favicon
  (letter fallback) above the related-news shelf; bookmarks and the grid open
  in a new tab instead of replacing the start page.
- Browser: core chrome — chat panel, top and workspace bars, virtual keyboard,
  navigation HUD, travel panel, tile overlay and shared popups — now punches
  through a native page's mask, with a watcher that clears a hole as soon as a
  panel hides (`web/js/nativeOccluders.js`).
- Browser: `runJs` view command and `page-js` event, so the shell can evaluate
  a script in a page and return its result to the AI bridge.
- Agent: tool artifacts are streamed as SSE `artifact` events as soon as they
  are saved; the UI adopts them immediately, so a window a tool opens exists
  before the next tool call in the same turn.
- Updates: every system upgrade now ends with a package-state audit and repair
  (`dpkg --audit`, then `dpkg --configure -a && apt-get -f install` on apt), so
  a failed kernel postinst cannot leave later apt commands broken.
- Updates: Ollama installs are verified before success is reported — zero-byte
  GPU libraries and a missing `llama-server` are detected, and the service unit
  the installer rewrites is restored.
- Radio: now-playing artwork via MusicBrainz and the Cover Art Archive, plus
  direct station streams, are allowed by the shell CSP; `all.api.radio-browser.info`
  replaces the dead `fi1` host.
- Greeter: `shiny-greeter-start` turns the expected xinit exit during a seat
  handover into success, so a normal login no longer marks the boot degraded
  or triggers the fallback unit.

### Fixed

- Browser: the chat panel and other core overlays were painted over by native
  pages because only the plugin's own popups were cut out of the mask.
- Browser: a bookmarks-panel click navigated the current tab and lost the
  start page.
- Updates: an interrupted Ollama update could leave zero-byte CUDA libraries
  and no `llama-server`, making Ollama fall back to CPU silently.
- Updates: the Ollama installer rewrote `ollama.service` with a temporary
  `systemctl` shim in `PATH`; the real unit is now restored.
- Updates: Ollama is skipped when the system upgrade fails, and the update
  refuses to start if the service cannot be stopped.
- Radio: missing CSP entries made the station grid and artwork fail with
  "Radio Browser unreachable", and external HTTP(S) streams were refused.
- Greeter: QtWebEngine ran with an unwritable profile from the repo and the
  login card was unreadably small; the greeter now uses its own data dir and a
  150% scale.
- T2 audio: the period-fix installer refuses to override an in-tree
  `t2bce_audio` 0.04+ driver, which broke later kernel postinstalls through
  DKMS.

## [0.1.1] - 2026-10-25

### Added

- Live image configures apt against the public LiteAura repository and registers
  the payload in dpkg, so `apt upgrade` tracks `liteaura` from the first boot.
- Native plugin windows follow updates: hidden windows rebuild silently, a
  visible window picks the new version when it is closed and reopened.
- Release publishing from `v*` tags: Debian package, signed APT suite `liteaura` on
  GitHub Pages and a GitHub Release with the package and checksums.

### Fixed

- Apple T2 live audio: WirePlumber no longer fails with a missing `liblua5.4`,
  and the T2 audio stack is configured before optional components.
- Craft store waits for connectivity in the live image instead of caching the
  GitHub error for an hour.
- Release build installs `libclang` for the `btls-sys` build script.

## [0.1.0] - 2026-10-25

### Added

- Initial LiteAura live installer ISO for Apple T2 and generic hardware.
- `liteaura` Debian package with the Qt shell, the `shiny` core, plugins and web UI.
- Signed APT repository served from GitHub Pages.
