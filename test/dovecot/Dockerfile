# Dovecot, built rather than pulled.
#
# The obvious thing is `image: dovecot/dovecot` in the compose file, and that is what this
# started as. It does not work on the machine hosting this: Docker Desktop's credential helper
# is consulted on every `pull`, it reads the login keychain, and a keychain cannot be unlocked
# from a non-interactive SSH session. The image is public and needs no credentials at all, but
# the helper runs regardless and fails before the request is made.
#
# Building from a base that is already on the host sidesteps the registry completely: `apk`
# fetches from Alpine's mirrors over plain HTTPS, which has nothing to do with Docker Hub.
#
# It is also the better dependency. `dovecot/dovecot:2.3-latest` is a moving tag, and a test
# rig whose server changes underneath it produces failures that are nobody's fault and take a
# day to attribute.
FROM alpine:latest

RUN apk add --no-cache dovecot dovecot-pigeonhole-plugin openssl ca-certificates \
    && adduser -D -u 1000 -h /srv/mail mailuser \
    && mkdir -p /srv/mail /etc/dovecot/certs /etc/dovecot/users \
    && chown -R mailuser:mailuser /srv/mail

# Alpine's package ships a full example configuration that would override ours. The rig
# supplies exactly one config file and wants nothing else read.
RUN rm -rf /etc/dovecot/conf.d && mkdir -p /etc/dovecot/conf.d

COPY dovecot.conf /etc/dovecot/dovecot.conf

EXPOSE 143 993

# Foreground, so Docker owns the process and `docker compose logs` is the whole story — the
# soak test counts connections from those logs.
CMD ["dovecot", "-F"]
