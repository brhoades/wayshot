# NOTES ABOUT FORK
This fork pulls in a multi-monitor patch from:

https://github.com/mstoeckl/wayshot

Which isn't yet upstream. It also adds a nix flake and exposes several pieces as a library crate.

<p align=center>
  <img src="https://git.sr.ht/~shinyzenith/wayshot/blob/main/docs/assets/wayshot.png" alt=wayshot width=60%>
  <p align=center>A native, blazing-fast 🚀🚀🚀 screenshot tool for wlroots based compositors such as sway and river written in Rust.</p>

  <p align="center">
  <a href="./LICENSE.md"><img src="https://img.shields.io/github/license/waycrate/wayshot?style=flat-square&logo=appveyor"></a>
  <img src="https://img.shields.io/badge/cargo-v1.2.2-green?style=flat-square&logo=appveyor">
  <img src="https://img.shields.io/github/issues/waycrate/wayshot?style=flat-square&logo=appveyor">
  <img src="https://img.shields.io/github/forks/waycrate/wayshot?style=flat-square&logo=appveyor">
  <img src="https://img.shields.io/github/stars/waycrate/wayshot?style=flat-square&logo=appveyor">
  <br>
  <img src="https://repology.org/badge/vertical-allrepos/wayshot.svg">
  </p>
</p>

# Some usage examples:

NOTE: Read `man 7 wayshot` for more examples.

NOTE: Read `man wayshot` for flag information.

Region Selection:

```bash
wayshot -s "$(slurp)"
```

Fullscreen:

```bash
wayshot
```

Screenshot and copy to clipboard:

```bash
wayshot --stdout | wl-copy
```

Pick a hex color code, using ImageMagick:

```bash
wayshot -s "$(slurp -p)" --stdout | convert - -format '%[pixel:p{0,0}]' txt:-|egrep "#([A-Fa-f0-9]{6}|[A-Fa-f0-9]{3})" -o
```

# Known bugs:

Multi monitor systems break on `--slurp` usage. This is quite the tricky bug and will need some refactoring which we're currently working on. (https://github.com/waycrate/wayshot/issues/7)

# Installation

## AUR:

`wayshot-git` & `wayshot-bin` have been packaged.

## Compile time dependencies:

-   scdoc (If present, man-pages will be generated.)
-   rustup
-   make

## Compiling:

-   `git clone https://github.com/waycrate/wayshot && cd wayshot`
-   `make setup`
-   `make`
-   `sudo make install`

# Support:

1. https://matrix.to/#/#waycrate-tools:matrix.org
2. https://discord.gg/KKZRDYrRYW

# Smithay Developers:

Massive thanks to smithay developer <a href="https://github.com/cmeissl">Cmeissl</a> and <a href="https://github.com/vberger">Victor Berger</a>. Without them this project won't be possible as my wayland knowledge is limited.
