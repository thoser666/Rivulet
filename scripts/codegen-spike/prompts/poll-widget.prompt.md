# Poll widget

Create a poll overlay: a centered card, 480 px wide, with a question line and
up to 4 answer rows. Each row shows label + horizontal percentage bar + count.
Bars animate width changes over 400 ms, colors: row 1 teal, row 2 orange,
row 3 violet, row 4 rose. Expose `window.rivuletPoll(question, options)` to
set a poll and `window.rivuletVote(index)` to increment. Seed a demo poll
"Next game?" with [Minecraft 12, Fortnite 7, Rocket League 3] on load.
