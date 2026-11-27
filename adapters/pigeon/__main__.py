# `python -m pigeon` and `python -m adapters.pigeon` both land here, from either side of this directory.

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from app import main

sys.exit(main(sys.argv[1:]))
