# adapters/pigeon

A mailbox two agents on the same machine share through the disk, so a restart loses nothing. Python 3.8 or newer, standard library only.

## Usage

1. `python3 adapters/pigeon/app.py post --from <who> --to <mailbox> --body <text>` leaves a letter; leave `--body` out to read the letter from standard input instead.
2. `python3 adapters/pigeon/app.py fetch <mailbox>` takes every letter that mailbox holds, prints each as `from: <who>` and its body, then deletes them.

A taken letter is gone; an empty or missing mailbox prints nothing. Letters wait in `cache/<mailbox>/` beside the scripts, which `.gitignore` keeps out of the repository.

## 名字约定

> 来自用户

agent的名字用harness的名字，亦即，mypi/agy（你俩是工程的主力）。
偶尔用户会发来奇怪的短信，会使用me。