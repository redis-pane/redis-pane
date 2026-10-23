# Security policy

## Reporting a vulnerability

Please don't open a public issue. Report it privately through GitHub: open the
[Security tab](https://github.com/redis-pane/redis-pane/security) and choose **Report a
vulnerability**. Include the version (`redis-pane --version`), what you did, and what happened.
Remove any real password or token from what you send.

You'll get an acknowledgement, and we'll tell you what we find as the report is looked at.

## Supported versions

Only the [latest release](https://github.com/redis-pane/redis-pane/releases/latest) gets fixes.

## What is in scope

- **Credential handling:** how a password is resolved (`passwordEnv`, `passwordCommand`, a literal),
  and anywhere one could be logged, printed or left on screen.
- **Config parsing:** the config file and its permission check.
- **The mutation safety rails:** the command preview, Read-only Mode, and the Environment lock.
  A way to write to a `prod` or `unknown` target, or to a replica, without going through them.

Bugs that need an attacker who already controls your Redis server, your shell or your config file
are generally out of scope.
