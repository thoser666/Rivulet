# Donation / tip goal bar

Create a donation goal bar overlay: a horizontal bar at the bottom of the
screen, 600 px wide, centered. It shows "Goal: <raised> / <target> EUR" and a
progress fill in animated green-to-gold gradient. Expose
`window.rivuletGoal(raised, target)` so the host can update it live; the bar
animates smoothly to new values. Start at 130 / 250 EUR. Style: dark glass
panel, rounded corners, subtle glow.
