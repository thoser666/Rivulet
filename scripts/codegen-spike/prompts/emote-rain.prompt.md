# Emote rain

Create an emote-rain overlay: emoji falling from the top of the screen to the
bottom, starting at random x positions, random sizes 24–64 px, gentle horizontal
sway while falling, rotation, and removal when off-screen. Density: start with
12 falling emotes from the set 🎉 ❤️ ⭐ 🔥 😂 and expose
`window.rivuletEmote(emoji)` to spawn more (limit 80 concurrently). Background
fully transparent, no scrollbars.
