# RepoSync

This project allows to synchronize any debian/redhat repository to s3+cloudfront configuration.
---

# CLI


## help
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

## To check the configuration
```
$ reposync my-config.yaml check
config file is correct
```

## To synchronize a repository directly
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

## To run in server mode
```
$ reposync my-config.yaml server
```

### Use the APIs
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

# Configuration
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
# arbytrary name of the repository, expect 'all', which is reserved
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
# optional public pgp key, to validate the signature
      public_pgp_key: |
        -----BEGIN PGP PUBLIC KEY BLOCK-----
        ....
        -----END PGP PUBLIC KEY BLOCK-----
    destination:
# s3 endpoint, either use AWS or custom
      s3_endpoint: https://s3.example.com/
# s3 bucket
      s3_bucket: "my-bucket"
# region name, if using AWS, use official names
      region_name: "custom"
# path where to copy the repisotiry to
      path: "/centos8/"
# optional cloudfront endpoint & ARN resource ID
      cloudfront_endpoint: https://cloudfront.example.com/
      cloudfront_arn: arn
# AWS credentials
      access_key_id: key
      access_key_secret: secret

```

## S3 & CloudFront endpoints

Check the official [S3 AWS endpoints](https://docs.aws.amazon.com/general/latest/gr/s3.html), [CloudFront AWS endpoints](https://docs.aws.amazon.com/general/latest/gr/cf_region.html), or provide a custom url.

## Build 
To build just run:
 ```cargo build --release```
 
## API & OpenAPI
`./update.sh` is used to generate the `reposync-lib` from the [openapi schema](generated/api/openapi.yaml).
 
See generated [README](generated/README.md).

To edit APIs [ApiCurio](https://studio.apicur.io/) was used.