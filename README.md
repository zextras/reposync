# RepoSync

This software mirrors Debian and RedHat repositories to either an S3 bucket,
with an optional CloudFront, or a local directory.

No special access is required from the source repository.

---

## Features
- RPM and DEB repository
- CHECKSUM validation
- signature validation
- one time mirroring
- server mode with anonymous API to trigger synchronizations
- high consistency
- single binary

## Use cases
We develop this software mainly for mirror repositories to S3/CloudFront, so we
could leverage the simplicity of having a low-load repository with the availability of a 
managed service.
Another use case is to mirror repositories locally, allowing deployments within a closed network.

---

## How it works

### TLDR
We ordered the operation specifically to reduce the effects when the synchronization is
interrupted, in short as long as source packages are not modified we can guarantee consistency.

### Steps
When a synchronization starts RepoSync downloads the indexes of the repository and compare them
with the current ones, which are empty in the beginning, it creates an operation for each new,
deleted, or modified package or index. It downloads one package at the time to reduce storage 
requirements, validate it, and then write it in the destination repository. The procedure 
writes and uploads indexes **after** all new packages are written, so every package referenced
in the new index will be available right away.
After the upload RepoSync invalidates modified files in CloudFront cache, if configured.
Then RepoSync deletes old indexes and removed packages, as new indexes are already available,
and as the final step it writes the new indexes locally the new indexes, which will be used on the
next synchronization.

If at any point during the synchronization a failure were to occur such as a checksum mismatch or a I/O
error, RepoSync interrupts the synchronization, and delete downloaded indexes.
For RepoSync the synchronization never happened, and will perform everything from scratch on the next 
iteration.

---

## Help
```
RepoSync 0.9
Keep a repository synchronized to an S3 bucket

USAGE:
    reposync [OPTIONS] <CONFIG_FILE> <ACTION>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
        --repo <REPO>    which repo to synchronize, check, sync, or server

ARGS:
    <CONFIG_FILE>    location of config file
    <ACTION>         action to perform, 'check', 'sync' or 'server'
```

## Check the configuration
```
$ reposync my-config.yaml check
config file is correct
```

## Synchronize directly a repository
```
$ reposync my-config.yaml sync --repo my-repo
starting synchronization of my-repo
requesting: https://repo.example.com/dists/xenial/Release
requesting: https://repo.example.com/dists/xenial/InRelease
requesting: https://repo.example.com/dists/xenial/Release.gpg
requesting: https://repo.example.com/dists/xenial/main/binary-amd64/Packages
....
requesting: https://repo.example.com/dists/bionic/test/binary-i386/Packages.gz
repo fully synchronized
```
_You can use `all` to synchronize all repositories._

## Run in server mode
```
$ reposync my-config.yaml server
```

### Call the APIs
```
$ wget --method=POST http://localhost:8080/repository/my-repo/sync -q -O - | jq .
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

---

## Config file

see [a sample config](samples/config.yaml) for a complete sample.
```
---
general:
# where to store every repository status
  data_path: "/data/repo/"
# used for temporary storage during synchronization
  tmp_path: "/tmp/"
# if run in server mode, where to bind the HTTP port to
  bind_address: "127.0.0.1:8080"
# timeout of HTTP requests
  timeout: 60
# max. amount of retries in case HTTP request fails
  max_retries: 3
# how many seconds to wait before trying again
  retry_sleep: 5
# refresh the repository at least every x minutes
  min_sync_delay: 5
# refresh the repository every x minutes, even if not requested
  max_sync_delay: 30
repo:
# arbytrary name of the repository, exept 'all', which is reserved
# multiple repositories can be specified
  - name: my-redhat-repo
# versions to fetch, only used for debian repositories
    versions:
      - xenial
      - bionic
      - focal
    source:
# either 'debian' or 'redhat' for deb or rpm repository
      kind: debian
# endpoint of the repository
      endpoint: https://my-repo.example.com/RHEL/8/x86_64/stable/
# optional username & password
      username: username
      password: password
# or authorization_file, expected format username:password
      authorization_file: /run/secrets/http_authorization
# optional public pgp key, to validate the signature
      public_pgp_key: |
        -----BEGIN PGP PUBLIC KEY BLOCK-----
        ....
        -----END PGP PUBLIC KEY BLOCK-----
    destination:
# only one destination must be specified, either local or s3
      local:
        path: "/my/repo/path"
      s3:
# s3 endpoint, either use AWS or custom
        s3_endpoint: https://s3.example.com/
# s3 bucket
        s3_bucket: "my-bucket"
# region name, if using AWS, use official names
        region_name: "custom"
# path where to copy the repisotiry to
        path: "/centos8/"
# optional cloudfront endpoint & ARN resource ID
        cloudfront_endpoint: https://cloudfront.amazonaws.com/
        cloudfront_distribution_id: id
# AWS credentials
        access_key_id: key
        access_key_secret: secret
# AWS credential file, expected format: {ACCESS_KEY_ID}\n{SECRET_ACCESS_KEY}
        aws_credential_file: /run/secrets/aws_credential


```

## S3 & CloudFront endpoints

Check the official [S3 AWS endpoints](https://docs.aws.amazon.com/general/latest/gr/s3.html), [CloudFront AWS endpoints](https://docs.aws.amazon.com/general/latest/gr/cf_region.html), or provide a custom url.

---
## Build 
To build just run:
 ```cargo build --release```
 
## API & OpenAPI
`./update.sh` is used to generate the `reposync-lib` from the [openapi schema](generated/api/openapi.yaml).
 
See generated [README](generated/README.md) for a complete list of HTTP APIs.

To edit APIs [ApiCurio](https://studio.apicur.io/) was used.