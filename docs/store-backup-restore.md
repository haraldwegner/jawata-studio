# Backing up and restoring the experience store

The experience store is one file:

    ~/.local/share/jawata/experience.mv.db

It holds every recorded experience on this machine and exists nowhere else.

## Where backups live

    ~/.local/share/jawata/backups/<timestamp>/experience.mv.db

One folder per backup, named by date and time. The engine additionally writes
`experience-pre-migration-v<N>.zip` beside the live file before each schema
migration; those are the migration ladder's own safety net, not a replacement
for these backups.

## Taking a backup

1. Stop the workspaces in jawata-studio (Stop all), so nothing is writing.
2. Copy the file:

       mkdir -p ~/.local/share/jawata/backups/$(date +%Y%m%d-%H%M%S)
       cp ~/.local/share/jawata/experience.mv.db ~/.local/share/jawata/backups/<that folder>/

3. Start the workspaces again.

## Restoring

1. Stop the workspaces.
2. Put the backup in place (keep the broken file, renamed, until you are sure):

       mv ~/.local/share/jawata/experience.mv.db ~/.local/share/jawata/experience.mv.db.broken
       cp ~/.local/share/jawata/backups/<folder>/experience.mv.db ~/.local/share/jawata/

3. Start the workspaces. Ask any workspace a question it should know the
   answer to (the Memory page, or `/memorize` recall in a client). If it
   answers, the restore worked.

## The rule that makes a backup real

**A backup that has never been restored is not a backup.** After taking one
that matters, restore it somewhere and make it answer a question. On
2026-08-21 exactly this drill caught nothing — and an earlier "backup" from
July (`experience-pre-migration-v3.zip`) turned out to be 22 bytes: an empty
archive that would have restored to nothing. Only the drill distinguishes the
two.

Same-disk limit: these backups guard against a bad migration or a corrupted
file, not against the disk itself dying. For that, copy a backup folder to a
second disk or a synced location of your choice.
