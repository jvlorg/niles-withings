# Withings add-on logo

`withings-logo.svg` is a monochrome vector trace of the white heart symbol in the owner-supplied Withings app-logo image. It is not an official vector source; the mark belongs to its respective owner and is used to identify this integration.

The transparent SVG uses `currentColor` and Niles' required `viewBox="0 0 24 24"`. `withings-logo-preview.png` shows the same geometry on light/dark backgrounds and at small sizes.

The build copies the SVG to `logo.svg`, as declared in the add-on manifest. Include `logo.svg` alongside `integration.wasm` and `niles-integration.toml` when publishing alpha.10. The real Niles 0.3 checker accepted the logo sanitizer and complete package, and disconnected component startup passed. No Niles frontend patch is needed or included in this add-on package.
