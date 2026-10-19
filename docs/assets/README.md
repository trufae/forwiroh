# Forwiroh logo

The mint and ivory interlocking endpoints represent two devices linked through
one private connection. This original mark was generated for Forwiroh with the
built-in image-generation tool. It uses the repository's MIT license.

Source: `forwiroh-logo.png`. The iOS AppIcon and Android launcher use the same
1024-pixel RGB export. Android applies its launcher mask; iOS applies the system
app-icon shape. The small Android notification symbol remains a separate
monochrome resource.

Generation prompt:

> Create an original, polished, minimal app icon for Forwiroh, a peer-to-peer
> networking app connecting your phone to your home computer. A single bold
> abstract symbol made from two interlocking rounded cable-like shapes suggesting
> both an F and an S, with two small circular endpoint cutouts. Friendly precise
> geometric silhouette, recognizable at 32 pixels. Flat two-tone mint and warm
> ivory mark, centered on a solid deep dark teal square background. Full bleed
> square image; no rounded outer corners, border, lettering, words, mockup,
> shadow, texture, gradients or padlock. Entire symbol inside the central 60
> percent of the canvas with generous equal margins, suitable for circular
> Android launcher masks and iOS app icons.

To reproduce the platform export with ImageMagick:

```sh
convert docs/assets/forwiroh-logo.png -resize 1024x1024 -alpha off \
  apps/ios/Forwiroh/Assets.xcassets/AppIcon.appiconset/AppIcon.png
cp apps/ios/Forwiroh/Assets.xcassets/AppIcon.appiconset/AppIcon.png \
  apps/android/app/src/main/res/drawable-nodpi/forwiroh_logo.png
python3 scripts/generate-xcode.py
```
