# Regenerating the README images

Everything here is recorded from the **real widget running in waybar**, in
`--demo` mode so no real spend or repository names are published.

## The bar animation (`bar.gif`)

cicdbar is not a TUI — it prints one line of JSON and exits — so a terminal
recording shows its least interesting face. The honest demo is the bar itself
changing, captured from the compositor.

1. Point the waybar module at a script that cycles the demo scenarios, and
   drop the interval to 1 second:

   ```sh
   #!/usr/bin/env bash
   # Time-based, not a counter: waybar runs the module once per output, so a
   # counter advances twice per tick and you only ever see half the states.
   N=$(( ( ($(date +%s) / 2) % 4 ) + 1 ))
   exec cicdbar --demo "$N" --format '{total_usd} · {run_glyph}{running} · {proj_pct}%'
   ```

2. Reload waybar (`killall -SIGUSR2 waybar`) and capture frames:

   ```sh
   for i in $(seq -w 1 30); do grim -o <OUTPUT> frames/full-$i.png; done
   ```

3. Crop to the module, keep one frame per state, and assemble:

   ```sh
   ffmpeg -framerate 3 -i seq/%03d.png \
     -vf "split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse" \
     -loop 0 bar.gif
   ```

4. **Restore the waybar config.**

## The tooltip (`tooltip.png`)

Hover the module and screenshot it. Programmatic cursor placement does not
raise a GTK tooltip — it needs real pointer *motion*:

```sh
swaymsg 'seat - cursor set <x> <y>'
swaymsg 'seat - cursor move 25 5'
```

Crop tightly: a full-screen capture will pick up whatever else is on screen.
