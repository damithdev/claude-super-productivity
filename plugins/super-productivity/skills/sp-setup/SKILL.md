---
name: sp-setup
description: First-run setup for the super-productivity plugin. Installs or updates Super Productivity with the user's consent and checks that the local REST API token exists. Use on /super-productivity:sp-setup or when sp-cli says SP is missing or has no token.
---

# SP setup

1. Ask the user for consent to install and auto-update Super Productivity. Wait for the answer.
2. If the answer is yes, run `sp-cli ensure --consent`.
   If the answer is no, stop. Say that `sp-cli` will not work until SP is installed.
3. Run `sp-cli ensure --now` with a 10-minute timeout (an install can take minutes).
4. Show the output to the user.
5. If the output says "SP REST API is off", tell the user to open Super Productivity, go to
   Settings > Misc and switch on "Local REST API". Then run `sp-cli ensure --now` again.
   As an alternative, the user can set the `SP_API_TOKEN` environment variable.
6. Run `sp-cli bridge-plugin --out "$HOME/sp-cli-bridge.zip"`. Tell the user to load that zip once
   in Super Productivity (Settings > Plugins, upload). Without it, `sp-cli add-project`, `add-tag`
   and link attachments fail; the other commands work.
7. Run `sp-cli list --project Inbox` to prove the connection.
8. Ask whether the user wants a continuous backup of their SP data. If yes, tell them to open
   Settings > Sync & Backup > Set up sync, pick the "Local file" provider, choose a folder inside
   their cloud-drive folder (for example OneDrive), and click Save & Enable Sync.
   Say this works only as a backup for one computer: never point SP on a second device at the
   same folder, because a synced copy from another device can overwrite the data. To sync several
   devices, use one of SP's own providers instead (SuperSync, Dropbox, WebDAV or OneDrive).
