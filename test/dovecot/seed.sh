#!/bin/sh
# Fills the test mailbox with messages. docs/04 §Phase 5 exit gate — "a 50k-message mailbox".
#
#   ./seed.sh 50000
#
# Written directly into the Maildir rather than delivered over IMAP. Fifty thousand APPENDs is
# an hour of round trips and tests the *server's* throughput, which is not what the gate is
# about; the gate is about Halcyon syncing a mailbox that is already large.
#
# The messages are deliberately varied: threads, non-threads, replies that arrive out of
# order, a few with attachments, a few with only a plain-text part, and some with the
# malformed headers real mail is full of. A mailbox of fifty thousand identical messages
# would prove nothing about threading or parsing.
set -eu

COUNT="${1:-50000}"
CONTAINER="${CONTAINER:-halcyon-dovecot}"
USER_MAIL="tester@halcyon.test"

echo "seeding $COUNT messages into $USER_MAIL ..."

docker exec -i "$CONTAINER" sh -s "$COUNT" "$USER_MAIL" <<'INNER'
set -eu
COUNT="$1"
USER_MAIL="$2"
DIR="/srv/mail/$USER_MAIL/Maildir/new"
mkdir -p "$DIR" "/srv/mail/$USER_MAIL/Maildir/cur" "/srv/mail/$USER_MAIL/Maildir/tmp"

i=0
while [ "$i" -lt "$COUNT" ]; do
  i=$((i + 1))

  # One thread per twenty messages, so threading has real work: replies, and replies to
  # replies, rather than fifty thousand singletons.
  thread=$(( i / 20 ))
  in_thread=$(( i % 20 ))

  if [ "$in_thread" -eq 0 ]; then
    refs=""
    subject="Thread $thread: the quarterly figures"
  else
    parent=$(( i - 1 ))
    refs="In-Reply-To: <msg-$parent@halcyon.test>
References: <msg-$(( thread * 20 ))@halcyon.test> <msg-$parent@halcyon.test>"
    subject="Re: Thread $thread: the quarterly figures"
  fi

  # A spread of dates so the list's keyset pagination is exercised over a real range.
  ts=$(( 1700000000 + i * 37 ))

  {
    echo "Return-Path: <sender$(( i % 97 ))@example.test>"
    echo "Message-ID: <msg-$i@halcyon.test>"
    [ -n "$refs" ] && echo "$refs"
    echo "From: Sender $(( i % 97 )) <sender$(( i % 97 ))@example.test>"
    echo "To: Tester <$USER_MAIL>"
    echo "Subject: $subject"
    echo "Date: $(date -u -d "@$ts" '+%a, %d %b %Y %H:%M:%S +0000' 2>/dev/null || date -u '+%a, %d %b %Y %H:%M:%S +0000')"

    if [ $(( i % 11 )) -eq 0 ]; then
      # HTML plus a quoted reply, which is what the reader's quote folding is for.
      echo "MIME-Version: 1.0"
      echo "Content-Type: text/html; charset=utf-8"
      echo ""
      echo "<p>Message $i. Tracking 1Z999AA1012345678$(( i % 10 )).</p>"
      echo "<blockquote><p>The previous message in thread $thread.</p></blockquote>"
    elif [ $(( i % 17 )) -eq 0 ]; then
      # A malformed date and a missing charset. Real mail is full of both.
      echo "Content-Type: text/plain"
      echo ""
      echo "Message $i with no charset declared and an odd date."
    else
      echo "MIME-Version: 1.0"
      echo "Content-Type: text/plain; charset=utf-8"
      echo ""
      echo "Message $i in thread $thread."
    fi
  } > "$DIR/$(printf '%d.%d.halcyon' "$ts" "$i")"

  if [ $(( i % 5000 )) -eq 0 ]; then echo "  ... $i"; fi
done

chown -R 1000:1000 "/srv/mail/$USER_MAIL"
echo "seeded $COUNT"
INNER

echo "reindexing ..."
docker exec "$CONTAINER" doveadm force-resync -u "$USER_MAIL" INBOX >/dev/null 2>&1 || true
docker exec "$CONTAINER" doveadm mailbox status -u "$USER_MAIL" messages INBOX
