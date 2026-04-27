# Output Safety Replay Report

Directory: `crates/edit_prediction_cli/evals-generated/native-captures/20260421-111437`
Format: `V0131GitMergeMarkersPrefix`

## Summary

- Cases: `65`
- Safe to apply: `26`
- Unsafe: `39`
- Parse failures: `0`
- Patch failures: `0`
- Sentinel leaks: `26`
- Giant deletions: `39`

## Cases

| Request | Case | Safe | Parse | Patch | Raw Bytes | Old Bytes | New Bytes | Deletion Ratio | Reasons |
| --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: | --- |
| `request-0001` | `captured-response` | `yes` | `yes` | `yes` | `1432` | `1004` | `1432` | `0.00` | none |
| `request-0001` | `identity-old-editable` | `yes` | `yes` | `yes` | `1004` | `1004` | `1004` | `0.00` | none |
| `request-0001` | `raw-sentinel-leak` | `no` | `yes` | `yes` | `53` | `1004` | `37` | `0.96` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1004 new_bytes=37 |
| `request-0001` | `giant-deletion` | `no` | `yes` | `yes` | `0` | `1004` | `0` | `1.00` | suspicious giant deletion: old_bytes=1004 new_bytes=0 |
| `request-0001` | `malformed-marker-span` | `no` | `yes` | `yes` | `53` | `1004` | `53` | `0.95` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1004 new_bytes=53 |
| `request-0002` | `captured-response` | `yes` | `yes` | `yes` | `1432` | `1004` | `1432` | `0.00` | none |
| `request-0002` | `identity-old-editable` | `yes` | `yes` | `yes` | `1004` | `1004` | `1004` | `0.00` | none |
| `request-0002` | `raw-sentinel-leak` | `no` | `yes` | `yes` | `53` | `1004` | `37` | `0.96` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1004 new_bytes=37 |
| `request-0002` | `giant-deletion` | `no` | `yes` | `yes` | `0` | `1004` | `0` | `1.00` | suspicious giant deletion: old_bytes=1004 new_bytes=0 |
| `request-0002` | `malformed-marker-span` | `no` | `yes` | `yes` | `53` | `1004` | `53` | `0.95` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1004 new_bytes=53 |
| `request-0003` | `captured-response` | `yes` | `yes` | `yes` | `1432` | `1004` | `1432` | `0.00` | none |
| `request-0003` | `identity-old-editable` | `yes` | `yes` | `yes` | `1004` | `1004` | `1004` | `0.00` | none |
| `request-0003` | `raw-sentinel-leak` | `no` | `yes` | `yes` | `53` | `1004` | `37` | `0.96` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1004 new_bytes=37 |
| `request-0003` | `giant-deletion` | `no` | `yes` | `yes` | `0` | `1004` | `0` | `1.00` | suspicious giant deletion: old_bytes=1004 new_bytes=0 |
| `request-0003` | `malformed-marker-span` | `no` | `yes` | `yes` | `53` | `1004` | `53` | `0.95` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1004 new_bytes=53 |
| `request-0004` | `captured-response` | `yes` | `yes` | `yes` | `1431` | `1003` | `1431` | `0.00` | none |
| `request-0004` | `identity-old-editable` | `yes` | `yes` | `yes` | `1003` | `1003` | `1003` | `0.00` | none |
| `request-0004` | `raw-sentinel-leak` | `no` | `yes` | `yes` | `53` | `1003` | `37` | `0.96` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1003 new_bytes=37 |
| `request-0004` | `giant-deletion` | `no` | `yes` | `yes` | `0` | `1003` | `0` | `1.00` | suspicious giant deletion: old_bytes=1003 new_bytes=0 |
| `request-0004` | `malformed-marker-span` | `no` | `yes` | `yes` | `53` | `1003` | `53` | `0.95` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1003 new_bytes=53 |
| `request-0005` | `captured-response` | `yes` | `yes` | `yes` | `1391` | `1056` | `1391` | `0.00` | none |
| `request-0005` | `identity-old-editable` | `yes` | `yes` | `yes` | `1056` | `1056` | `1056` | `0.00` | none |
| `request-0005` | `raw-sentinel-leak` | `no` | `yes` | `yes` | `53` | `1056` | `37` | `0.96` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1056 new_bytes=37 |
| `request-0005` | `giant-deletion` | `no` | `yes` | `yes` | `0` | `1056` | `0` | `1.00` | suspicious giant deletion: old_bytes=1056 new_bytes=0 |
| `request-0005` | `malformed-marker-span` | `no` | `yes` | `yes` | `53` | `1056` | `53` | `0.95` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1056 new_bytes=53 |
| `request-0006` | `captured-response` | `yes` | `yes` | `yes` | `1378` | `1043` | `1378` | `0.00` | none |
| `request-0006` | `identity-old-editable` | `yes` | `yes` | `yes` | `1043` | `1043` | `1043` | `0.00` | none |
| `request-0006` | `raw-sentinel-leak` | `no` | `yes` | `yes` | `53` | `1043` | `37` | `0.96` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1043 new_bytes=37 |
| `request-0006` | `giant-deletion` | `no` | `yes` | `yes` | `0` | `1043` | `0` | `1.00` | suspicious giant deletion: old_bytes=1043 new_bytes=0 |
| `request-0006` | `malformed-marker-span` | `no` | `yes` | `yes` | `53` | `1043` | `53` | `0.95` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1043 new_bytes=53 |
| `request-0007` | `captured-response` | `yes` | `yes` | `yes` | `1379` | `1044` | `1379` | `0.00` | none |
| `request-0007` | `identity-old-editable` | `yes` | `yes` | `yes` | `1044` | `1044` | `1044` | `0.00` | none |
| `request-0007` | `raw-sentinel-leak` | `no` | `yes` | `yes` | `53` | `1044` | `37` | `0.96` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1044 new_bytes=37 |
| `request-0007` | `giant-deletion` | `no` | `yes` | `yes` | `0` | `1044` | `0` | `1.00` | suspicious giant deletion: old_bytes=1044 new_bytes=0 |
| `request-0007` | `malformed-marker-span` | `no` | `yes` | `yes` | `53` | `1044` | `53` | `0.95` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1044 new_bytes=53 |
| `request-0008` | `captured-response` | `yes` | `yes` | `yes` | `1380` | `1045` | `1380` | `0.00` | none |
| `request-0008` | `identity-old-editable` | `yes` | `yes` | `yes` | `1045` | `1045` | `1045` | `0.00` | none |
| `request-0008` | `raw-sentinel-leak` | `no` | `yes` | `yes` | `53` | `1045` | `37` | `0.96` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1045 new_bytes=37 |
| `request-0008` | `giant-deletion` | `no` | `yes` | `yes` | `0` | `1045` | `0` | `1.00` | suspicious giant deletion: old_bytes=1045 new_bytes=0 |
| `request-0008` | `malformed-marker-span` | `no` | `yes` | `yes` | `53` | `1045` | `53` | `0.95` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1045 new_bytes=53 |
| `request-0009` | `captured-response` | `yes` | `yes` | `yes` | `1381` | `1046` | `1381` | `0.00` | none |
| `request-0009` | `identity-old-editable` | `yes` | `yes` | `yes` | `1046` | `1046` | `1046` | `0.00` | none |
| `request-0009` | `raw-sentinel-leak` | `no` | `yes` | `yes` | `53` | `1046` | `37` | `0.96` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1046 new_bytes=37 |
| `request-0009` | `giant-deletion` | `no` | `yes` | `yes` | `0` | `1046` | `0` | `1.00` | suspicious giant deletion: old_bytes=1046 new_bytes=0 |
| `request-0009` | `malformed-marker-span` | `no` | `yes` | `yes` | `53` | `1046` | `53` | `0.95` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1046 new_bytes=53 |
| `request-0010` | `captured-response` | `yes` | `yes` | `yes` | `1383` | `1048` | `1383` | `0.00` | none |
| `request-0010` | `identity-old-editable` | `yes` | `yes` | `yes` | `1048` | `1048` | `1048` | `0.00` | none |
| `request-0010` | `raw-sentinel-leak` | `no` | `yes` | `yes` | `53` | `1048` | `37` | `0.96` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1048 new_bytes=37 |
| `request-0010` | `giant-deletion` | `no` | `yes` | `yes` | `0` | `1048` | `0` | `1.00` | suspicious giant deletion: old_bytes=1048 new_bytes=0 |
| `request-0010` | `malformed-marker-span` | `no` | `yes` | `yes` | `53` | `1048` | `53` | `0.95` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1048 new_bytes=53 |
| `request-0011` | `captured-response` | `yes` | `yes` | `yes` | `1385` | `1050` | `1385` | `0.00` | none |
| `request-0011` | `identity-old-editable` | `yes` | `yes` | `yes` | `1050` | `1050` | `1050` | `0.00` | none |
| `request-0011` | `raw-sentinel-leak` | `no` | `yes` | `yes` | `53` | `1050` | `37` | `0.96` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1050 new_bytes=37 |
| `request-0011` | `giant-deletion` | `no` | `yes` | `yes` | `0` | `1050` | `0` | `1.00` | suspicious giant deletion: old_bytes=1050 new_bytes=0 |
| `request-0011` | `malformed-marker-span` | `no` | `yes` | `yes` | `53` | `1050` | `53` | `0.95` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1050 new_bytes=53 |
| `request-0012` | `captured-response` | `yes` | `yes` | `yes` | `1386` | `1051` | `1386` | `0.00` | none |
| `request-0012` | `identity-old-editable` | `yes` | `yes` | `yes` | `1051` | `1051` | `1051` | `0.00` | none |
| `request-0012` | `raw-sentinel-leak` | `no` | `yes` | `yes` | `53` | `1051` | `37` | `0.96` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1051 new_bytes=37 |
| `request-0012` | `giant-deletion` | `no` | `yes` | `yes` | `0` | `1051` | `0` | `1.00` | suspicious giant deletion: old_bytes=1051 new_bytes=0 |
| `request-0012` | `malformed-marker-span` | `no` | `yes` | `yes` | `53` | `1051` | `53` | `0.95` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1051 new_bytes=53 |
| `request-0013` | `captured-response` | `yes` | `yes` | `yes` | `1387` | `1052` | `1387` | `0.00` | none |
| `request-0013` | `identity-old-editable` | `yes` | `yes` | `yes` | `1052` | `1052` | `1052` | `0.00` | none |
| `request-0013` | `raw-sentinel-leak` | `no` | `yes` | `yes` | `53` | `1052` | `37` | `0.96` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1052 new_bytes=37 |
| `request-0013` | `giant-deletion` | `no` | `yes` | `yes` | `0` | `1052` | `0` | `1.00` | suspicious giant deletion: old_bytes=1052 new_bytes=0 |
| `request-0013` | `malformed-marker-span` | `no` | `yes` | `yes` | `53` | `1052` | `53` | `0.95` | raw sentinel leaked into applied text; suspicious giant deletion: old_bytes=1052 new_bytes=53 |
