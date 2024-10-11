# What is this?

This is the configuration that I use to run a personal Apt proxy using
`approx`. I have several Debian machines and containers running at any given
time, and I run a local cache to reduce the bandwidth to my preferred Apt
mirror.

# How do I use it?

## Requirements

You need two things:

1. A container manager (duh). I use Podman but everything here should work with
   Docker as well. If you do use Docker make sure to run `make DOCKER=docker`
   instead of just `make`.

2. Cargo. The host program is written in Rust so you need to be able to build
   it. The current stable release should be fine, the only dependency is libc.

## Configuring the cache

First, you need to know what Apt repositories you want to cache. Caching debian
and debian-security is a good starting point, and you may want to cache other
third-party repositories depending on how often you use them.

Start by looking at the `.sources` file for the repository:

```bash
$ cat /etc/apt/sources.list.d/debian.sources
Types: deb
URIs: http://deb.debian.org/debian
```

Make up a name for this repository and add the name and URL a file called
`approx.conf`. This file must live beside the Dockerfile at the root of this
repository. (The names here are deliberately bad, ideally you would call these
repos `debian` and `debian-security`, but that will make things confusing later).

```bash
$ echo 'hitchhiker http://deb.debian.org/debian' > approx.conf
$ echo 'gargle-blaster http://deb.debian.org/debian-security' >> approx.conf
```

## Build the image

```bash
# Use Podman to build an image and tag it 'apt-cache'
$ make 

# If you want to use Docker
$ make DOCKER=docker

# If you want to use a different tag
$ make IMAGE_TAG=approx
```

## Run the container

By default, the container listens on port 80. I recommend creating a bind mount
to hold the cached packages:

```bash
$ podman run -d -p 80:80 -v apt-cache:/var/cache/approx apt-cache
```

### Environment Variables

The container supports some environment variables for configuration:

- `CACHE_PORT` What port the container listens on for connections to the Apt
  proxy. By default this is port 80.

The other two variables are related to package cleanup. approx does not remove
old packages automatically, which may lead to lots of space being used on
packages that you are no longer using. This container includes a process that
sweeps the package cache periodically and removes files that have not been
updated within a set time.

These variables are given as `DD:HH:MM:SS` intervals, where you only specify
the components below the largest unit. So `10:00:00:00` is 10 days, `10:00:00`
is 10 hours, `10:00` is 10 minutes, and `10` is 10 seconds.

- `GC_INTERVAL`: How long to wait between cleanups. Note that this is only
  tracked while the container is alive, if you restart the container before
  this interval has elapsed no cleanup occurs. Defaults to `06:00:00`.

- `GC_MAXAGE`: How old a file must be before it is deleted. Defaults to
  `30:00:00:00`.

## Using the container

Once the container is running, you need to update the Apt configuration for
each machine that you want to use it. The root of each repository is proxied
using the name that you gave in approx.conf.

Going back to the stock debian.sources we were looking at before, we can patch
it to use `http://my-apt-cache/hitchhiker` for the main repository:

```bash
$ cat /etc/apt/sources.list.d/debian.sources
Types: deb
URIs: http://deb.debian.org/debian
...
$ sed -i 's@http://deb.debian.org/debian@http://my-apt-cache/hitchhiker@g'
$ cat /etc/apt/sources.list.d/debian.sources
Types: deb
URIs: http://my-apt-cache/hitchhiker
...
```

# Internals

## What is approx_host?

The `approx` Apt cache expects some things from its runtime environment that
containers don't provide by default:

- `/dev/log`: `approx` likes to writes messages to syslog, but there's no
  default syslog listener. `approx_host syslog` creates a Unix domain socket on
  `/dev/log` and dumps incoming messages to stdout.

- inetd: `approx` expects to be run from an inetd environment, where its stdin
  and stdout are hooked up to TCP sockets. `approx_host inetd <port> <command> <args...>`
  listens on the given port and forks off a child process that runs the given
  command and arguments, connecting the socket to the process's standard IO
  file descriptors.

`approx_host` also implements the garbage collection service discussed in the
**Environment Variables** section above.
