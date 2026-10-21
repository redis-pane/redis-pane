# A bastion in a box: sshd, redis-pane, and a Redis that only listens on the
# container's own loopback. The single way to reach that Redis is to ssh in and
# run redis-pane there, which is the deployment the product is built for (PRD
# §1). Driven by scripts/ssh-box.sh; built from the repository root.
#
# The binary is built from the checkout, so this tests the code in front of you,
# not the last release. It is a musl build rather than the shipped
# x86_64-unknown-linux-gnu one: the TUI does not care, and it keeps the runtime
# image on the same redis:8.4-alpine that scripts/redis-up.sh pins (ADR-0006's
# tracking behaviour is verified against 8.4.0).

FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p redis-pane \
 && cp target/release/redis-pane /usr/local/bin/redis-pane

FROM redis:8.4-alpine
# tmux is here because "over SSH, inside tmux" is the common bastion setup and
# the one where OSC 52 copying most often goes missing (ADR-0013).
RUN apk add --no-cache openssh-server python3 tmux \
 && adduser -D -s /bin/sh pane \
 && passwd -u pane \
 && echo 'set -g set-clipboard on' > /home/pane/.tmux.conf \
 && chown pane:pane /home/pane/.tmux.conf
COPY --from=build /usr/local/bin/redis-pane /usr/local/bin/redis-pane
COPY scripts/resp.py scripts/fixtures.py scripts/churn.py /opt/redis-pane-scripts/
COPY scripts/ssh-box/sshd_config /etc/ssh/sshd_config.d/redis-pane.conf
COPY scripts/ssh-box/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN printf '#!/bin/sh\nexec python3 /opt/redis-pane-scripts/fixtures.py "$@"\n' > /usr/local/bin/fixtures \
 && printf '#!/bin/sh\nexec python3 /opt/redis-pane-scripts/churn.py "$@"\n' > /usr/local/bin/churn \
 && chmod +x /usr/local/bin/fixtures /usr/local/bin/churn /usr/local/bin/entrypoint.sh
EXPOSE 22
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
