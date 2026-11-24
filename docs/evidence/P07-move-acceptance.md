# Mailbox moves and Undo

The private Wayland session exercised the actual reader toolbar, row context menu,
searchable destination picker and Undo notification against three loopback Dovecot
configurations. It used the combined versions of the [folder picker](https://github.com/eduardz1/mailstronaut/pull/12),
[row menu](https://github.com/eduardz1/mailstronaut/pull/15) and
[persistent warning](https://github.com/eduardz1/mailstronaut/pull/17).

| Server capabilities | Observed behavior |
| --- | --- |
| MOVE and UIDPLUS | The filtered destination queued its original IMAP wire name. Escape and account reload cleared the choice. Undo kept the message in its original folder beyond the grace period. The row menu changed flags only on the target and preserved the neighboring reader. Archive and Trash reached their server folders. |
| UIDPLUS only | COPY and UID EXPUNGE removed the selected message. An unrelated message already marked for deletion stayed in the inbox with its original flag. |
| Neither capability | The inbox message was restored, the abandoned move was never retried, and new mail arrived. The account warning survived a successful sync and restart. Dismissing it persisted through a second restart. All five originals remained. |

The test command ran `experiments/p02-accounts/credentials.sh` with
`MAILSTRONAUT_ACTIONS_GUI_BINARY` and `MAILSTRONAUT_CREDENTIAL_FIXTURE` set to
the pinned development binaries. [Combined sources used for the run](https://github.com/Heavenlystorm/mailstronaut/tree/31cc4b873a3a60306ea4c611c58a709895f2d732).
