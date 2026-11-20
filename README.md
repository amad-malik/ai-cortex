**DO NOT INSTALL.** As of 20 November 2026 the installer of Cortex 0.2.1 and 0.2.2 does not work on Rocky Linux 10.2 or 10.3. Cortex 0.2.3, planned for 26 November 2026, replaces both.

The text below is kept as it was; its install command does not work on Rocky Linux 10.2 or 10.3.

# Cortex

**Cortex is a system for handing work between AI workers with a record of what was asked, what came back and who accepted it.** Work arrives as
a *handoff* — an objective, what counts as done, and what evidence is needed. A worker claims it, returns it with evidence, and a reviewer
accepts it or sends it back. Everything a worker learns is written down as a **record** that can be found again, and nothing is edited in
place: a record is superseded or invalidated, so the history is kept.

**This repository is the public home of Cortex**, and it is where the release and the install line below point. **The source code of Cortex is not published with the 0.2.1 application**: the download
contains the built program and the server image, not the source. Where GitHub shows "Source code" files on the public home, they hold this
README and the licence. The application is distributed
under the **MIT licence** (see the licence line below and `LICENSE`).

## Install Cortex 0.2.1 (Linux)

Cortex 0.2.1 runs on **Rocky Linux 10, x86_64** and on nothing else yet. Copy and paste this one line into a terminal on the machine, logged in
as your normal user with sudo — **not as root, and not through `sudo -u`**:

```sh
curl --proto '=https' --tlsv1.2 -fsSLo install.sh https://github.com/amad-malik/ai-cortex/releases/download/v0.2.1/install.sh && sh ./install.sh --project cortex-1 --lead lead1 --release 0.2.1 --base-url https://github.com/amad-malik/ai-cortex/releases/download/v0.2.1 --yes
```

`cortex-1` and `lead1` are examples — replace them with your own names (a small letter first, then small letters, digits and hyphens, 32 characters
at most), and use the same lead name in the later lines of the release text, because it is the name of your sign-in file.

**Do not pipe it.** `curl … | sh` stops half way and can still look like it worked. The line above downloads the installer to a file and runs
**that file**.

To see that it works:

```sh
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8601/v2/health/ready      # 200
```

## What we tested, and the limits

**Tested, with the same signed files, from the release's previous address on 13 Nov:** on one fresh, stock Rocky Linux 10.2 machine, with no Cortex
and none of the container prerequisites on it, that line **in its previous-address form** installed Cortex with **no question asked**; a second run
changed nothing; it came back
by itself after a restart; and signing in, adding a second worker, creating, claiming and returning a handoff, accepting it, writing a record and
finding it by a word were all exercised.

**From this address: installed once, on 14 Nov** - status 0 on the screen, first try, 204 seconds, no question asked, and the installer downloaded from here is byte-identical to the signed one. A second run, a restart, the full check map and the first-hour walk-through were **not** part of it.

**What was not part of that test — and every limit you will meet — is in the release text itself**, on the release page for `v0.2.1` at the public
home named below. Read it
before you install: it names what the test did not cover, and the limits a user meets — thirteen of them — in plain words.

## Problems and questions

- **Create a GitHub issue on this repository.** That is how problems and questions reach us.

## One known defect in the release metadata

**The signed release's own metadata still names the old release page** as its notes address. That address is dead; **the release page on the
public home is the working one.** It is corrected in the next release.

## Licence

**MIT.** The Cortex application is distributed under the MIT licence. **The source code of Cortex is not published with the 0.2.1 application.** The MIT licence covers the application as we distribute it; the components it contains and the packages it installs keep their own
licences, which the release's notices file records.
