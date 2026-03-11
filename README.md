# RepoSync

Mirrors Debian and RedHat repositories to an S3 bucket (with optional CloudFront invalidation) or a local directory.

No special access is required from the source repository.

## Features

- DEB and RPM repository mirroring
- Checksum and PGP signature validation
- One-shot sync or continuous server mode with HTTP API
- Dry-run mode to preview changes without writing
- CloudFront cache invalidation on sync
- Cross-collection aware diffing (safe multi-distro pool handling)
- Single statically-linked binary

## How it works

Operations are ordered to minimize the impact of interrupted syncs. As long as source packages are not modified mid-sync, consistency is guaranteed.

### Sync steps

1. Download source repository indexes and diff against the last known state.
2. For each new or modified package: download, validate checksum, write to destination.
3. Upload new indexes **after** all packages are written — every package referenced in the new index is already available.
4. Invalidate modified paths in CloudFront cache (if configured).
5. Delete old indexes and removed packages.
6. Persist the new indexes locally for the next sync cycle.

If any step fails (checksum mismatch, I/O error, etc.) the sync is aborted and local state is not updated — the next run starts from scratch.

## Requirements

- Rust >= 1.91 (aws-sdk crates requirement)
- OpenSSL development headers (for building)

## Build

```
make build          # debug
make release        # release
make test           # run tests
make lint           # format check + clippy
make help           # show all targets
```

Or directly with cargo:

```
cargo build --release
```

### Docker

```
make docker-build
```

Uses a multi-stage Alpine build (`deployment/Dockerfile`). The final image contains only the static binary, `ca-certificates`, `libgcc`, and `openssl`.

## Usage

```
RepoSync 0.11.0
Keep a repository synchronized to an S3 bucket

USAGE:
    reposync [OPTIONS] <CONFIG_FILE> <ACTION>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information
        --dry-run    Show what would be done without making any changes

OPTIONS:
        --repo <REPO>    Which repo to synchronize

ARGS:
    <CONFIG_FILE>    Location of config file
    <ACTION>         Action to perform: 'check', 'sync', or 'server'
```

### Check the configuration

```
$ reposync my-config.yaml check
config file is correct
```

### Synchronize a repository

```
$ reposync my-config.yaml sync --repo my-repo
starting synchronization of my-repo
requesting: https://repo.example.com/dists/jammy/Release
requesting: https://repo.example.com/dists/jammy/InRelease
requesting: https://repo.example.com/dists/jammy/Release.gpg
requesting: https://repo.example.com/dists/jammy/main/binary-amd64/Packages
....
my-repo fully synchronized
```

Use `--repo all` to synchronize all configured repositories.

### Dry-run

```
$ reposync my-config.yaml sync --repo my-repo --dry-run
[DRY-RUN] would copy: pool/main/p/package_1.0_amd64.deb
[DRY-RUN] would delete: pool/main/o/old-package_0.9_amd64.deb
my-repo dry-run complete
```

### Run in server mode

```
$ reposync my-config.yaml server
starting http server, listening on 127.0.0.1:8080
```

#### HTTP API

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Health check (returns 200 or 503) |
| `GET` | `/repository/{repo}` | Repository sync status |
| `POST` | `/repository/{repo}/sync` | Trigger a synchronization |

Example:

```
$ curl -s -X POST http://localhost:8080/repository/my-repo/sync | jq .
{
  "name": "my-repo",
  "status": "syncing",
  "next_sync": 1622477603603,
  "last_sync": 1622477303603,
  "last_result": "successful",
  "size": 15750859220,
  "packages": 369
}
```

## Configuration

See [samples/config.yaml](samples/config.yaml) for a complete example.

```yaml
general:
  data_path: "/data/repo/"         # repository state storage
  tmp_path: "/tmp/repo/"           # temporary storage during sync
  bind_address: "127.0.0.1:8080"   # server mode listen address
  timeout: 60                      # HTTP request timeout (seconds)
  max_retries: 3                   # retry count on HTTP failure
  retry_sleep: 5                   # seconds between retries
  min_sync_delay: 5                # minimum minutes between syncs
  max_sync_delay: 30               # auto-sync interval (minutes)

repo:
  - name: my-repo
    versions:                      # distro versions (debian only)
      - jammy
      - noble
    source:
      kind: debian                 # 'debian' or 'redhat'
      endpoint: https://repo.example.com/
      # optional auth (pick one)
      username: user
      password: pass
      authorization_file: /run/secrets/http_authorization  # format: user:pass
      # optional PGP signature validation
      public_pgp_key: |
        -----BEGIN PGP PUBLIC KEY BLOCK-----
        ...
        -----END PGP PUBLIC KEY BLOCK-----
    destination:
      # local destination
      local:
        path: "/var/lib/reposync/my-repo/"
      # or S3 destination
      s3:
        s3_endpoint: https://s3.us-east-1.amazonaws.com/
        s3_bucket: my-bucket
        region_name: us-east-1
        path: "/my-repo/"
        access_key_id: AKIA...
        access_key_secret: secret
        aws_credential_file: /run/secrets/aws_credential  # format: KEY_ID\nSECRET
        # optional CloudFront invalidation
        cloudfront_endpoint: https://cloudfront.amazonaws.com/
        cloudfront_distribution_id: E1234567890
```

### S3 and CloudFront endpoints

See [S3 endpoints](https://docs.aws.amazon.com/general/latest/gr/s3.html) and [CloudFront endpoints](https://docs.aws.amazon.com/general/latest/gr/cf_region.html), or provide a custom URL for S3-compatible storage.

## License

AGPL-3.0 — see [LICENSE](LICENSE).
