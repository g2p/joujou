# joujou: play a music album on a chromecast audio

joujou takes a directory of music files (sorted by file names) and sends
it to a chromecast for playback.

Playback control (volume control, navigation within the playlist) is
done via an application like Google Home.

joujou takes metadata from the music files, including covers if embedded.
When embedded covers are not found, joujou looks for image files placed
next to the music files (cover.jpg for example); this heuristic only
works when all files come from a single album.

joujou serves the files to the chromecast over the local network.
You may have to whitelist a range of ports that should be connectable
from your local network range and configure joujou to use them (through
the --ports flag).  Music files are accessed by the chromecast directly,
but cover files can be accessed by other devices, for example phones
used to control playback.

Installation is done via cargo.
Make sure rust is installed, then run:

    cargo install --git https://github.com/g2p/joujou.git

