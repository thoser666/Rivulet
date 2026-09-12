# Chat box

Create a chat overlay: the 5 most recent chat messages stacked bottom-left,
each fading in with a slide-from-left animation and auto-fading out after 30
seconds. Username in bold cyan, message in white, on a semi-transparent dark
pill. Expose `window.rivuletChat(user, message)` to push messages. Seed three
demo messages on load ("StreamFan: Lets gooo!", "PixelPete: hi from Germany",
"ModMarc: rules in pinned msg").
