# Feature: BucketFS Client

Lets a Rust caller list, upload, download, and delete files in an Exasol BucketFS bucket through the exarrow-rs crate, so applications and CLIs share one BucketFS client instead of each writing its own.

## Background

* The client exists only when the crate is built with the `bucketfs` Cargo feature.
* BucketFS serves a bucket over HTTP or HTTPS at `<scheme>://<host>:<port>/<bucket>/<path>`, and a GET on `<bucket>/` returns the bucket's file paths, one per line.
* BucketFS accepts HTTP Basic auth with the user `r` and the read password, or the user `w` and the write password.
* The client returns results and typed errors to the caller and prints nothing.

## Scenarios

### Scenario: Connection settings take defaults from the host alone

* *GIVEN* a caller creates BucketFS connection settings from a host name only
* *WHEN* the caller reads the settings
* *THEN* the port SHALL be 2581
* *AND* the bucket SHALL be `default`
* *AND* TLS SHALL be on
* *AND* server certificate validation SHALL be on
* *AND* the settings SHALL hold no read password and no write password

### Scenario: List the immediate children of a path

* *GIVEN* a bucket holds the files `a/x.txt`, `a/b/y.txt`, and `top.txt`
* *WHEN* the caller lists the path `a` without the recursive option
* *THEN* the client SHALL return `b` and `x.txt`, sorted, each once
* *AND* listing the empty path SHALL return `a` and `top.txt`

### Scenario: List every file under a path recursively

* *GIVEN* a bucket holds the files `a/x.txt`, `a/b/y.txt`, and `top.txt`
* *WHEN* the caller lists the path `a` with the recursive option
* *THEN* the client SHALL return `b/y.txt` and `x.txt`, sorted, relative to `a`

### Scenario: A list prefix matches whole path segments

* *GIVEN* a bucket holds the files `data/x.txt` and `database/y.txt`
* *WHEN* the caller lists the path `data`
* *THEN* the client SHALL return only `x.txt`
* *AND* the client MUST NOT return an entry under `database`

### Scenario: Listing a path with no files fails

* *GIVEN* a bucket holds no file under the path `missing`
* *WHEN* the caller lists the path `missing`
* *THEN* the client SHALL return a not-found error that names `missing`
* *AND* listing the empty path of an empty bucket SHALL return an empty list without error

### Scenario: Upload a local file

* *GIVEN* a local file `driver.jar` exists and the connection settings hold a write password
* *WHEN* the caller uploads `driver.jar` to the destination `drivers/`
* *THEN* the client SHALL store the file at `drivers/driver.jar` in the bucket with an HTTP PUT
* *AND* a destination that does not end with `/` SHALL be the stored path as given
* *AND* an upload to a path that already holds a file SHALL replace that file's content

### Scenario: Upload from an async reader streams the body

* *GIVEN* the connection settings hold a write password
* *AND* the caller has an async reader whose data is larger than one read
* *WHEN* the caller uploads from the reader to a bucket path
* *THEN* the client SHALL send each piece of the body to BucketFS as it reads it, before the reader reaches its end
* *AND* the stored file SHALL hold the reader's bytes unchanged

### Scenario: Download a file to a local path

* *GIVEN* a bucket holds the file `drivers/driver.jar`
* *WHEN* the caller downloads `drivers/driver.jar` to a local path
* *THEN* the client SHALL write the file content to that path
* *AND* a local path that is an existing directory SHALL receive the file as `driver.jar` inside it

### Scenario: Download into an async writer streams the body

* *GIVEN* a bucket holds a file larger than one HTTP body piece
* *WHEN* the caller downloads the file into an async writer
* *THEN* the client SHALL write each body piece to the writer as it arrives
* *AND* the writer SHALL receive the file's bytes unchanged

### Scenario: Delete a file

* *GIVEN* a bucket holds the file `old.txt` and the connection settings hold a write password
* *WHEN* the caller deletes `old.txt`
* *THEN* the client SHALL send an HTTP DELETE for `old.txt`
* *AND* a later listing of the bucket MUST NOT contain `old.txt`

### Scenario: Download or delete of a missing file fails with not found

* *GIVEN* a bucket holds no file at `nope.txt`
* *WHEN* the caller downloads or deletes `nope.txt`
* *THEN* the client SHALL return a not-found error that names `nope.txt`
* *AND* a download MUST NOT create or change the local destination

### Scenario: A read chooses its credential from the settings

* *GIVEN* a list or download request
* *WHEN* the client sends the request
* *THEN* it SHALL authenticate as `r` with the read password when the settings hold one
* *AND* it SHALL authenticate as `w` with the write password when the settings hold only a write password
* *AND* it SHALL send no Authorization header when the settings hold neither password

### Scenario: A write without a write password fails before any request

* *GIVEN* the connection settings hold no write password
* *WHEN* the caller uploads or deletes a file
* *THEN* the client SHALL return a missing-write-password error
* *AND* the client MUST NOT send an HTTP request

### Scenario: Rejected credentials fail with an authentication error

* *GIVEN* BucketFS answers a request with HTTP 401 or 403
* *WHEN* the caller runs any BucketFS operation
* *THEN* the client SHALL return an authentication error
* *AND* any other unsuccessful HTTP status SHALL return an error that names the status

### Scenario: An unreachable service fails with a connection error

* *GIVEN* the connection settings point at a host and port where no service accepts connections
* *WHEN* the caller runs any BucketFS operation
* *THEN* the client SHALL return an unreachable error that names the host and port

### Scenario: Parse a BucketFS URI

* *GIVEN* a path argument `bfss://default/a/b.txt`, `bfs://default/a/b.txt`, or `a/b.txt`
* *WHEN* the caller parses it as a BucketFS URI
* *THEN* the result SHALL hold the in-bucket path `a/b.txt` for each form
* *AND* the result SHALL hold the bucket `default` for both URI forms and no bucket for the plain path
* *AND* the result SHALL ask for TLS for `bfss://`, ask for no TLS for `bfs://`, and hold no TLS preference for the plain path
* *AND* `bfs://default` and `bfs://default/` SHALL hold the empty in-bucket path

### Scenario: Passwords never appear in output

* *GIVEN* connection settings with a read password and a write password
* *WHEN* the caller formats the settings with Debug, or formats any error the client returns
* *THEN* the output MUST NOT contain either password
