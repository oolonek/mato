# Mato run {{RUN_ID}}
Report format: 1

Status: {{STATUS}}
Dataset DOI: {{DOI}}

Started (Unix seconds): {{STARTED}}
Finished (Unix seconds): {{FINISHED}}
Selected stages: {{STAGES}}
Metadata selection: {{SELECTION}}

## Dataset state at finish

Counts are for the configured dataset unless marked database-wide. “Not available” means the corresponding stage or database relation is absent; it does not mean zero.

{{METRICS}}

## Input and output files

{{FILES}}

## Failed files needing reprocessing

The latest failed annotation attempt for each file is listed once. “New this run” distinguishes this invocation from earlier outstanding failures.

{{FAILURES}}

## Inspect the database

```sh
{{COMMANDS}}
```

## Pipeline error

{{ERROR}}
