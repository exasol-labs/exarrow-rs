# Development Container Setup

This dev container uses Docker Compose to run both the development environment and an Exasol database.

## Services

* `dev-con`: Development container
* `dev-exa-db`: Exasol database container

## `dev-exa-db` (Exasol Database)
- Image: `exasol/docker-db:2025.2.0-arm64dev.0`
- Port: `8563`
- Default credentials:
  - User: `sys`
  - Password: `exasol`
- Runs in privileged mode (required by Exasol)

Run ARM-based Exasol container on macOS:

```bash
docker run --name exasoldb \
            -p 8563:8563 \
            -d \
            --privileged \
            exasol/docker-db:2025.2.0-arm64dev.0
```

See [exasol/docker-db](https://hub.docker.com/r/exasol/docker-db) for more details.

## Connecting to Exasol

From within the dev container, connect to Exasol using:
- Host: `dev-exa-db`
- Port: `8563`

From the host machine:
- Host: `localhost`
- Port: `8563`

## Starting the Environment

```bash
docker compose up -d
```

## Starting the firewall

Within the container run:
```bash
sudo /usr/local/bin/init-firewall.sh
```