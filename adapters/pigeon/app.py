# The command line: leave one letter in a mailbox, or take every letter a mailbox holds.

import argparse
import sys

from letters import fetch, post


def main(argv):
    parser = argparse.ArgumentParser(prog="pigeon", description="A mailbox both agents on this machine share.")
    words = parser.add_subparsers(dest="cmd", required=True)
    leaving = words.add_parser("post", help="leave one letter for a mailbox")
    leaving.add_argument("--from", dest="sender", required=True, metavar="NAME", help="who writes it")
    leaving.add_argument("--to", dest="recipient", required=True, metavar="NAME", help="whose mailbox takes it")
    leaving.add_argument("--body", metavar="TEXT", help="the letter itself; read from stdin when left out")
    taking = words.add_parser("fetch", help="take every letter a mailbox holds, then empty it")
    taking.add_argument("name", metavar="NAME", help="whose mailbox to empty")
    args = parser.parse_args(argv)

    if args.cmd == "post":
        post(args.sender, args.recipient, args.body if args.body is not None else sys.stdin.read())
        return 0

    for letter in fetch(args.name):
        print(f"from: {letter['from']}")
        print(letter["body"].rstrip("\n"))
        print()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
