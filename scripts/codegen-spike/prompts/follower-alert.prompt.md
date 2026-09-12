# Follower alert

Create an overlay that shows a follower alert: a box in the top-right corner
pops in with a springy animation, says "New follower: <name>" with a purple
gradient background, and automatically hides again after 4 seconds. Expose a
JavaScript function `window.rivuletAlert(name)` that triggers it, so the host
app can call it. Include a demo alert 1 second after page load.
