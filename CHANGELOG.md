# What changed in Zenith

Patch notes for every version, newest first. The section for a version is
what its release on GitHub says and what the app shows under "What is new",
so it is written for the people using Zenith, and to be worth reading: a
line at the top that says what the version is about, then what they will
notice, most important first, each point led by a few words in bold that
say it. Short and easy to take in: one line a point where that is
enough, plain words, and only what someone using Zenith will notice. True
to what the version does. A release is not built without one.

## 0.12.0

**A safer, steadier Zenith, and a better first start.** A top-to-bottom check of the code turned up ways to lose things; this version closes them.

### Getting started
- **A first start in three short steps.** Fix what stands in the way (only if something does), pick your clip key, and choose what goes into a clip, your microphone included.
- **Fixes, not instructions.** A recorder that is not running gets a "Start it" button.
- **An empty library says what to do.** Which key to press, how far back a clip reaches, and how many of your games Zenith already knows.
- **A pointer at your first clip** to what you can do with it, once.
- **See the first steps again** from Settings.

### Your clips and settings
- **An unplugged drive no longer wipes names and tags.** When the clip folder is on a drive that is not there, Zenith leaves the library alone until it is back.
- **Renaming cannot replace another clip.** A new name that only differs in capitals from another clip's gets a number instead.
- **One bad setting no longer resets the rest.** Zenith keeps every setting it can read, and saves a copy of the old file before changing it.
- **Closing while exporting asks first.** Nothing half-made is left behind.

### Recording
- **Linux: the clip key works without setup.** Pick it in Settings and your desktop holds it for Zenith. Where the desktop cannot, binding `zenithd save` works as before.
- **Windows: sound stays in step.** A stutter no longer shifts the sound for the rest of the session.
- **Windows: a game that changes resolution still gives a clip that plays.**
- **Windows: a full disk is said plainly**, and so is a clip that could not be saved, at once instead of after a wait.
- **A recorder that keeps stopping tells you**, once, instead of restarting quietly forever.
- **A save in progress finishes** when you sign out or quit.

### Editor and app
- **Trim handles no longer jump** when you start dragging them.
- **Sync changes undo in one step**, like volume.
- **A failed update keeps Zenith open** and says why, instead of just closing.
- **Thumbnails stop piling up.** Old ones are cleared once they pass 512 MB.

## 0.11.0

**The editor has a real player bar.**

- **Play is in the middle.** Frame back, previous cut and play from the start are on its left; frame forward, next cut and jump to the end are on its right.
- **Sync buttons move 100 ms.** Scroll over the number for fine steps of 10 ms. Click the number to reset it to 0.
- **Zoom buttons moved.** Minus and plus for the timeline are now in the bar above it, next to the volume.
- **"Back to the clip as recorded" is gone.** Use undo (Ctrl+Z) to step back instead.

## 0.10.0

**The editor went on a diet.** The tools beside the picture are gone: everything you cut with lives on the timeline now, and the side is one calm column with export, the clip, and the rest of your library.

### The big stuff
- **One side panel, no tabs.** Export on top, the clip under it, your library under that, all the way down the window. Nothing to switch between and nothing folded away.
- **Hop from clip to clip.** The library is right there in the editor as a list you can scroll, newest first, with a picture of each clip. Click one and it opens in place of the one you had, no trip back to the library.
- **The edit buttons are back over the tracks.** Split, Remove part, "start here" and "end here", undo, redo, and back to the clip as recorded, in one row between the picture and the timeline, where your eyes already are.
- **Sync sits beside the video track.** Minus and plus move the sound 10 ms, the wheel does the same (100 ms with Shift), the arrow keys work when it has the keyboard, and a click on the number puts it back to 0.
- **Save frame is a button.** The frame you are looking at, as a PNG beside the clip and on the clipboard, without a tool to find first.

### Looks
- **Titles that match.** The editor's side is headed the way the settings are, its facts sit on a card, and the clips in it look like the rows of the library's sidebar.
- **Zenith's icon in your colours.** Pick your own colours and the icon in the taskbar and the window switcher follows; on Linux the launcher's does too.
- **A wide screen gives the side more room.** On a big monitor the picture cannot use all the width anyway, so the side panel takes some.
- **The window keeps its outline** when it fills the screen.

### Smoother
- **Sizing the window drags less along.** The drawn sound stretches with the timeline while you drag the window's edge and is drawn again once you let go, and the library's cards mostly move apart instead of all being laid out again. This one is hard to measure from here: if it still hitches for you, Zenith now writes how drawing went to its log after every resize, so "Copy report" has the numbers.

### Windows
- **Updates say how they went.** A new `update.log` beside the other logs tells whether an update went in behind Zenith's own small window or had to fall back to the grey setup window, and why.
- **The update helper brings what it needs.** It runs from another folder than Zenith's, and now takes the runtime libraries it depends on along, so it starts wherever Zenith itself does.
- **The button that fills the screen, once more.** Another way it could end Zenith is closed.

### Gone, on purpose
- **The Cut, Sound and Frame tools.** With them went typing a part's exact start and end, "Keep only this", hearing one track alone, and the list of keys. Splitting, trimming, removing, the volumes and the sync are all on the timeline; the keys still work as before (S, Q, W, Delete, the arrows, Ctrl+Z), minus 1 to 4.

## 0.9.0

**The editor grew up.** The side of the editor is a proper set of tools now, the timeline runs the whole width of the window, and a friend's first evening with Zenith turned into a long list of fixes.

### The big stuff
- **Four tools beside the picture.** Clip, Cut, Sound and Frame: pick one at the end of the timeline, by its name over the panel, or with 1 to 4. Each shows what it is for as plain lines, and the numbers in colour can be dragged sideways, clicked to type, or stepped with the arrow keys.
- **Cut to the frame.** The Cut tool lists the parts of the clip and lets you set where one starts and ends by dragging its time. "Start here" and "End here" (Q and W) trim the part under the playhead to the playhead, and "Keep only this" throws the rest away in one go.
- **Save a frame as a picture.** The Frame tool steps through the clip a frame, a second or a cut at a time (the up and down arrows jump between cuts), and saves the frame you are on as a PNG beside the clip, copied and ready to paste.
- **Hear one track alone.** In the Sound tool the headphones on a track mute the others, and again brings them back. A dot by a tool's name says something in it is not as recorded.
- **The timeline has the whole window.** Export moved to the corner: the name, where it has to fit, and the button, in three lines instead of half the side. Split, trim, remove, undo and redo sit at the timeline's end, and zoom where the ruler begins.

### Asked for, done
- **Sync that moves.** One arrow moves the sound 10 ms, two move it 100 ms, and it now reaches three seconds either way. It says "300 ms later" instead of leaving you to work out the sign.
- **The wheel fine-tunes a volume.** One notch is 1%, on a track's volume and on the playback volume, and a whole turn of the wheel is one step to undo.
- **A gear for the settings.** The old icon did not look like settings to anyone.
- **Zenith on GitHub.** A link under Updates in the settings.
- **The keys moved out of the way.** The list of shortcuts is in the Clip tool, and no longer squeezed under the timeline.

### Windows
- **No more desktop at the start of a clip.** When capture runs only while a game does, nothing is recorded until the game's window is on screen. A game whose window never shows as one is recorded as the monitor after two minutes, so there is still something to clip.
- **The button that fills the screen.** For some, pressing it ended Zenith. What it does is now asked of Windows the way Windows' own button asks. If it still happens to you, the next point is for you.

### When something goes wrong
- **The window keeps a log.** Until now only the recorder did, so a crash of the window left nothing behind. It is kept from one start to the next, says when Zenith did not close as it should, and when it goes down it writes where it was.
- **"Copy report" remembers.** The report has the window's log in it, reaching back over the starts before this one, and more of the recorder's.

### Squashed bugs
- **Sizing the window stuttered.** The library was cutting its whole list again at every step of the window being sized, and making every card anew where one more fits beside the others, even while the editor or the settings were in front. It does neither now.

## 0.8.0

**Every launcher is invited.** Zenith on Windows now knows your games wherever they came from, Sync shows you what it is doing, and a round of locks went on behind the scenes.

### The big stuff
- **Not on Steam? Not a problem.** On Windows, games from Epic Games, GOG Galaxy, Ubisoft Connect, Battle.net, the EA app, Riot, Rockstar and Xbox (Game Pass) are found by themselves, just as Steam's are. Zenith captures while one runs, files its clips under its name, and records the game's own window. A game none of them knows can still be added in Settings → Games.
- **See your Sync.** The drawn sound of a clip now slides along with its Sync, so you can see where the sound lands against the picture, not only hear it.
- **Sync from the keyboard.** Tab to the number, the arrow keys move the sound a step, Delete puts it back.

### Windows glow-up, part two
- **Snap layouts.** On Windows 11, rest the pointer on the button that fills the screen and Windows offers its places to snap the window to, as on any other window.
- **The "Edit" button waits for you.** A clip saved during a game still leaves its notification in Windows' notification centre, with "Edit" on it, for when you are out of the game. The notice over the game tells you there and then.

### Safer under the hood
- **A video is only ever a video.** A file is read as the file it is: one that points to an address on the network, or to another file, is not followed there, by the library or by the player.
- **Updates are checked twice.** On Windows a downloaded update waits in Zenith's own folder instead of the shared temporary one, and is checked against its signature again right before it runs.
- **"Copy report" leaves your name out.** Your home folder is written as `~`.

### Squashed bugs
- **The line under "Custom size" in the export list was cut off.**
- **Light backgrounds were hard on the eyes.** Page names, the chosen quality, tags, the setup check's marks and the drawn sound are darkened on a light window so they can be read.

### Tidied up
- **No more white box at the bottom.** The notice that says "Copied." or offers "Undo" is in the window's own colours.

## 0.7.0

**Your clips, your mix.** Every clip now keeps its own volumes, sound that has drifted can be nudged back into place, and on Windows Zenith finally looks like it lives there.

### The big stuff
- **Every clip gets its own mix.** Turn the game down in one clip and the others stay exactly as they were. Choose where each track starts in Settings → Editing → "Initial volume" (Game, Voice chat, Microphone), and every fresh clip opens there.
- **Sound out of step? Nudge it.** "Sync" beside the timeline slides a clip's sound earlier or later against the picture, 10 ms a press. The mouse wheel on the number works too, and a click on it puts it back. You hear it straight away, and the export keeps it.
- **Names up front.** A clip in the library is headed by its own name, with its game and the game's cover underneath. A clip you never named is headed by when it was saved ("Today 10:17").
- **A percentage on every volume slider.** No more guessing whether that was 80 or 120.
- **Patch notes, whenever you like.** Settings → Updates → "What is new in" → Show. There is a fair chance that is where you are reading this.

### Windows glow-up
- **"Clip saved", right over your game.** Windows used to hold the notification back until you left the game. Zenith now shows a small notice of its own in the top right corner while you play. Outside a game the notification is as before, with its "Edit" button. (A game in old-school "exclusive fullscreen" lets no other program draw over it: switch it to borderless to see the notice.)
- **One bar instead of two.** Zenith's own bar is the title bar now, as on Linux: minimise, fill the screen and close are in it, drag it to move the window, double-click it to fill the screen. Snapping to the sides, the shadow and the rounded corners are still Windows' own. Start with `--system-frame` if you want Windows' title bar back.
- **Updates that look the part.** A new version goes in behind a small Zenith window in the colours you chose, not the grey setup window. One catch: the update to this version is still run by the version you have, so you get the old window one last time.

### Squashed bugs
- **Zenith quit on some game names.** A game added to Steam by hand with a name in another script (Chinese or Japanese, say) closed Zenith on the spot. Not any more.
- **The mouse wheel crawled through Settings.** One notch now moves the page a proper step.
- **Cut-off colour names.** The label that appears over a colour in Settings was clipped by its box. Labels like it are now drawn over everything, and go above when there is no room below.
- **Long game names in the list on the left were cut off.** They take a second line now, and one that is still too long shows in full when the pointer rests on it.
- **The tag that would not leave.** A clip put back the way it was kept its "Edited" tag.
- **Names stay names.** A window's title in "Add a game", or a file's name, is shown as it is written and never read as formatting.
- **An update fetches no more than the release says its file holds.**

### Tidied up
- **"Edited" is now "Trimmed"**, and it is only there while part of the clip is cut away. Moving a volume slider no longer puts it there.
- **Exports are called what the clip is called.** No export size tacked on the end. (A second export of the same clip gets a number, so the first is not written over.)
- **The export list matches the rest of the window** instead of being a white box, and the clip's name sits right above the export button.
- **Settings uses the whole window:** two columns when there is room.
- **"Sound against picture" is now "Audio synchronization"** in Settings → Sound.
- **The volumes 0.6.0 carried from clip to clip are gone**, now that each clip has its own. Set where tracks start under Settings → Editing.

## 0.6.0

### New
- The editor remembers your volumes. Set a track's volume (Game, Voice chat, Microphone) in one clip and the next clip you open starts with that track at the same volume. The playback volume beside the timeline is remembered too. "Back to the clip as recorded" still puts every track at 100%.
- Your own background and line colours: Settings → Colours has "Background" and "Lines" beside the highlight and button colours. Boxes and fields follow the background, and the words turn dark on a light one.
- Rename a clip beside the export: the Export panel has a "Name" field, as well as the name over the picture.
- Linux: the window has no title bar from the system any more. Zenith's own bar at the top has the buttons to minimise, fill the screen and close; drag the bar to move the window, double-click it to fill the screen, and drag the window's edges to size it. Start with `--system-frame` to keep the system's title bar.

### Fixed
- Moving a track's volume slider while the clip plays made the sound stutter. The sound now follows the slider without a gap.
- Playback ran on past the end of a clip you had trimmed, and past a split. It now stops at the end of the part it is in; play again goes on with the next part, and after the last part starts from the first. Enter still plays the whole result in one go.
- A clip renamed in the editor was exported under its old name.

### Changed
- A full quality export opens as it is, without the remembered volumes on top of the ones already in its sound.

## 0.5.0

### New
- Windows: Zenith records the game's own window instead of the whole screen, so nothing else on your monitor ends up in a clip. It finds the window of a Steam game, or of a game you added yourself, switches to it when the game starts and back to the screen when it closes. A windowed game that is not the shape of your monitor gets black bars beside it. Games from other launchers are recorded as before, with the whole screen, until you add them in Settings → Games.
- Add a game Zenith does not find by itself: Settings → Games → "Add a game". Start the game, pick its program from the list, and from then on Zenith captures while it runs and files its clips under the name you give.
- Windows: number pad keys can be the clip key (for example Alt and the number pad's minus). The number pad's digits work with Num Lock on or off.
- Your own colours: Settings → Colours changes the highlight colour and the colour of buttons.
- Windows: a game started without Steam saying so (through its own program or a launcher) is still noticed when it is one of your installed Steam games.

### Fixed
- Windows: choppy sound in clips. The recorder could miss sound while it waited for the video, lost sound of single programs when it read them a moment late, and could start a track with nothing to spare. All three are fixed, and the sound is now read ahead of the game when the game uses every processor.
- Windows: sound that came late against the picture. The sound of the moment the recorder was starting was kept and pushed everything after it back. If you moved the sound in Settings → Sound ("Sound against picture") to make up for it, set it back to 0 and check a clip.
- Windows: with two voice programs open at once, the voice track skipped.
- Windows: "Show" after an export opened nothing, or the wrong folder.
- Windows: the tray's "Open the clip folder" opened the wrong folder when the clip folder had been picked in Settings.
- Windows: an update switched "Start with Windows" back on.
- Windows: uninstalling with Zenith's window open left files behind.
- Windows: a clip saved while another program was reading it could be reported as not saved although it was.
- Windows: capture went on after Steam crashed while a game was running.
- Windows: a monitor unplugged and plugged in again no longer ends the recorder; it finds the monitor again.
- A game whose name ends in a dot, or is a name Windows keeps for itself, gets a folder that works.

### Changed
- Windows: the sound in clips is about a hundredth of a second later than before, which is what keeps it from skipping.
- Windows: Shift cannot be combined with a number pad digit as the clip key, and Ctrl cannot be combined with Scroll Lock: Windows hands those over as other keys, so they never worked.

## 0.4.0

### New
- A faster library. Only the clips on screen are drawn, so a library of thousands opens and scrolls as quickly as a small one.
- Select several clips at once (the box on a clip, Ctrl or Shift and a click, or Ctrl+A) and favourite, tag, move to another game, copy or delete them together.
- Undo for delete: a notice offers "Undo" before the clips go to the trash. More than 20 at once are asked about first.
- Tags and notes on clips. Search finds them (`#tag` searches tags only), and tags are listed in the sidebar.
- Sort clips by date, length or size.
- Drop video files onto the window to add them to the library; clips in folders inside folders are found too.
- Renaming a clip can rename its file as well.
- The recorder's status says how it is recording: point at the status in the top bar to see the capture method and encoder. The dot turns amber when Zenith had to fall back to a slower way.
- "Copy report" in Settings → Setup check gathers versions, the setup check, what the recorder is doing and its last log lines, to paste to whoever is helping. `zenithd report` prints the same.
- Windows: a tray icon. It shows what the recorder is doing and offers "Save clip", "Open Zenith", the clip folder, capture on/off/automatic and quitting the recorder; clicking it opens Zenith.
- Windows: notifications come from Zenith, with its name and icon, instead of appearing under Windows PowerShell.
- Windows: pick the clip key by pressing it. Click the key in Settings → Shortcut, then press the combination you want; Zenith says at once if it cannot be used.
- Windows: a "Start with Windows" switch in Settings.
- Windows: "Sound against picture" in Settings → Sound moves the sound earlier or later, for PCs where clips come out with the two apart.

### Changed
- Windows: uninstalling Zenith now also removes its buffer, logs and thumbnails. Your clips, library and settings stay.
- "What is new" in the app shows a version's full patch notes.
- Linux: the tray menu now reads as it does on Windows ("Save clip", "Open Zenith", "Quit recorder").

## 0.3.1

### New
- Zenith has its own icon: in the app, the launcher, notifications, and on Windows the taskbar, Start Menu and setup program.

### Fixed
- Games you added to Steam yourself ("non-Steam games") are now noticed, so capture starts and clips work in them. They are named as in Steam, without the ".exe", and use the cover you gave them there.

## 0.3.0

### New
- Recording on Windows. Zenith keeps the last minute while a game runs and saves a clip when you press Alt+F9; the key can be changed in Settings. The recorder starts when you sign in.
- Game sound, voice chat and microphone are recorded on separate tracks on Windows too.
- Choose which speakers and microphone are recorded, in Settings → Sound. A chosen device that is unplugged gives way to the default one.
- A PC with no usable graphics-card capture still records, by a slower method.

### Fixed
- Exporting a clip to a size (Discord, Nitro) failed on Windows. It works now.

## 0.2.0

### New
- Zenith for Windows: the clip library, the editor and export, with a setup program. (Recording on Windows came in 0.3.0.)
- Zenith updates itself: a button appears in the top bar when a new version is out. Updates are signed, tried before they replace the running version, and undone by themselves if they do not start.

## 0.1.0

The first version, for Linux: a background recorder that keeps a rolling buffer through gpu-screen-recorder while a game runs, a clip library, an editor that trims and cuts without touching the original, and export to a size that fits Discord.
