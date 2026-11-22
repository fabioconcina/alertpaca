## Fewer false CPU alerts

- **The CPU check reports the average since the previous run.** It used to
  measure a 200 ms window, so a run that started together with other cron
  jobs could read 98% on an idle machine and send a Critical alert. On
  Linux, alertpaca now keeps the CPU time counters from `/proc/stat` in
  `cpu.json` and reports the average usage since the last run, for example
  `3% avg over 5m (8 cores)`. Short spikes average out, and a CPU that stays
  busy between runs still alerts.

- **The 200 ms sample is still the fallback.** It is used on the first run,
  after a reboot, when the previous run was less than 10 seconds or more
  than 30 minutes ago, and on macOS.
