# joujou: play music files on a chromecast

joujou takes a directory of music files (sorting them by path) and sends
it to a chromecast for playback.

Playback control (volume control, navigation within the playlist) is
done via an application like Google Home.

joujou takes metadata from the music files, including covers if embedded.
When embedded covers are not found, joujou looks for image files placed
next to the music files (cover.jpg for example); this heuristic only
works when all files come from a single album.

joujou serves the files to the chromecast over the local network.
joujou defaults to listening on a random TCP port, but if you have a
firewall, you can pass the `--ports start[:end]` flag and configure
the firewall to allow access from your local network.  Music files are
accessed by the chromecast directly, but cover files can be accessed by
other devices on the same network, for example phones used to control
playback.

Supported codecs are: FLAC, MP3, Vorbis, Opus, AAC.
Vorbis and Opus can be in Ogg or WebM/Matroska containers, the rest
use their native container.  MP3 files should have ID3v2, although
joujou falls back to ID3v1 if needed.

Supported file extensions: .flac .mp3 .ogg .opus .oga .mka .m4a

This matches the formats a chromecast audio supports, with the exception
of WAV due to limited metadata support; use FLAC for lossless files.

## Usage

    joujou play path/to/album

Use

    joujou --help
    joujou play --help

for futher options.

Currently, there are flags for passing a beets metadata database and
starting past the first track.

## Installing

Use cargo to install joujou.
Make sure rust is installed (use [rustup] if in doubt, or look for a
package providing `cargo`), then run:

    cargo install --git https://github.com/g2p/joujou.git


[rustup]: https://rustup.rs
