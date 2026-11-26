# `python -m telegram` and `python -m adapters.telegram` both land here, from either side of this directory.
#
# The modules beside this file reach for each other by bare name, so the package's own directory is what goes
# on the path; the copy under `adapters/` is never imported as a package, and `telegram/` in a package never
# shadows the Telegram API module the bot talks to.

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from app import main

sys.exit(main(sys.argv[1:]))