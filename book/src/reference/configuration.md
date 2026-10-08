# Configuration

`redis-pane` reads `~/.config/redis-pane/config.json`. It never writes it. Unknown fields are
refused, so a misspelled `passwordEnv` is an error and not a password silently dropped.

A complete example with two Profiles:

```json
{
  "defaultProfile": "local",
  "profiles": {
    "local": {
      "host": "127.0.0.1",
      "port": 6379,
      "env": "local"
    },
    "staging": {
      "url": "rediss://cache-01.staging.example.com:6380/0",
      "username": "ops",
      "passwordEnv": "STAGING_REDIS_PASSWORD",
      "env": "staging",
      "note": "shared staging cache"
    }
  },
  "ascii": false,
  "theme": "dark"
}
```

A misspelled field is refused:

```json,invalid
{
  "profiles": {
    "staging": {
      "host": "cache-01.staging.example.com",
      "passwrodEnv": "STAGING_REDIS_PASSWORD"
    }
  }
}
```
