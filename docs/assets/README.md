# Forwiroh logo

The mark is an `F` and a `W` drawn as a single ethernet cable, and it is a
monogram you can trace with your eye: the cable leaves an RJ45 plug at the
bottom left, climbs the F stem, runs along the F's top arm, and that arm turns
straight into the first downstroke of the W. Through the W's two valleys it
finally rises and leaves as the arrowhead — the end of the cable is also the
end of the word.

Same idea underneath: one private connection carried end to end. The F's
crossbar is the only branch (a cable junction), and the plug sits on a single
baseline with the W's valleys so nothing floats.

`forwiroh-logo.svg` is the single source of truth. It is MIT licensed, like the
rest of the repository. Every raster asset is generated from it by
`scripts/render-logo.sh`:

```sh
scripts/render-logo.sh          # write all rasters
scripts/render-logo.sh --check  # CI: fail if a raster is stale
```

It renders one 1024-pixel RGB master and derives from it:

| output | consumer |
| --- | --- |
| `docs/assets/forwiroh-logo.png` | README hero image |
| `apps/ios/Forwiroh/Assets.xcassets/AppIcon.appiconset/AppIcon.png` | iOS app icon |
| `apps/android/app/src/main/res/drawable-nodpi/forwiroh_logo.png` | Android launcher |

The master is fully opaque; iOS applies the system app-icon shape and Android
applies its launcher mask, so the mark is sized and optically centred inside the
safe area both masks keep. The measured ink box is 759 by 570 units, centred at
(506, 513), with its furthest corner 479 units from the canvas centre — inside
the 512-unit radius a circular mask can cut. The Android notification symbol is
a separate monochrome vector drawable.

Colours: teal `#013C42` (gradient `#064E52` to `#01272C`), mint cable `#6BE0AC`
to `#A6F7CD`, ivory plug `#FAF5E3`. Flat shapes only, no strokes thinner than 24
units at 1024, so the mark survives downscaling to a launcher icon.
