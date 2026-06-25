# Test Cases

## Description
* This document contains several tables with test cases
* First column contains current time in format YYYY-MM-DD HH:MM:SS
* Second column is actor's type: USER or SYSTEM
* When actor is USER, third column contains user's input in chat like "12:45 call Poly", test framework should parse it as EventInfo and than map it to StoredEvent (now it's current StoredEvent)
* When actor is SYSTEM, test framework call function storage::play_at with current StoredEvent and current time from first column. Now returned value is new current StoredEvent. And current event's next_datetime must be equals to the time in third column (format YYYY-MM-DD HH:MM:SS). If after call play_at new StoredEvent.active==false then third column must be NONE.
* Fourth column holds the pure reminder message. When actor is USER it must equal the parsed `event.message`; when actor is SYSTEM it is left empty.
* Fifth column holds the canonical normalized time expression. When actor is USER it must equal `event.normalize_time()` (the re-parseable canonical form of the time/recurrence — e.g. `12:2` → `12:02`, `8 call Alex` → `08:00`, `mon-fri` → `Mon-Fri`); when actor is SYSTEM it is left empty. It is also left empty on USER rows whose input does not parse.
* A literal `\n` in the Input or Message column is decoded to a real newline (a markdown table cell cannot hold a line break), so multiline messages can be expressed.

## Cases

### Case 1: Future today — event fires once

| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 10:00:00 | USER   | 12:45 call Poly       | call Poly | 12:45      |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 12:45:00   |           |            |
| 2026-02-20 12:45:01 | SYSTEM | NONE                  |           |            |

### Case 1.2: Future today - only last event from previous table
| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 10:00:00 | USER   | 12:45 call Poly       | call Poly | 12:45      |
| 2026-02-20 12:45:01 | SYSTEM | NONE                  |           |            |

### Case 2: Past today — time already passed, fires the next day

| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 14:00:00 | USER   | 12:45 call Poly       | call Poly | 12:45      |
| 2026-02-20 14:00:00 | SYSTEM | 2026-02-21 12:45:00   |           |            |
| 2026-02-21 12:45:01 | SYSTEM | NONE                  |           |            |

### Case 2.2: Past today — first and last only
| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 14:00:00 | USER   | 12:45 call Poly       | call Poly | 12:45      |
| 2026-02-21 12:45:01 | SYSTEM | NONE                  |           |            |

### Case 3: Exact time — 12:45:00 equals now, not strictly future, fires next day

| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 12:45:00 | USER   | 12:45 call Poly       | call Poly | 12:45      |
| 2026-02-20 12:45:00 | SYSTEM | 2026-02-21 12:45:00   |           |            |
| 2026-02-21 12:45:01 | SYSTEM | NONE                  |           |            |

### Case 3.2: Exact time — first and last only
| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 12:45:00 | USER   | 12:45 call Poly       | call Poly | 12:45      |
| 2026-02-21 12:45:01 | SYSTEM | NONE                  |           |            |

### Case 4: One second before — barely future, fires today

| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 12:44:59 | USER   | 12:45 call Poly       | call Poly | 12:45      |
| 2026-02-20 12:44:59 | SYSTEM | 2026-02-20 12:45:00   |           |            |
| 2026-02-20 12:45:01 | SYSTEM | NONE                  |           |            |

### Case 4.2: One second before — first and last only
| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 12:44:59 | USER   | 12:45 call Poly       | call Poly | 12:45      |
| 2026-02-20 12:45:01 | SYSTEM | NONE                  |           |            |

### Case 5: Midnight — fires at 12:45 the same day

| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 00:00:00 | USER   | 12:45 call Poly       | call Poly | 12:45      |
| 2026-02-20 00:00:00 | SYSTEM | 2026-02-20 12:45:00   |           |            |
| 2026-02-20 12:45:01 | SYSTEM | NONE                  |           |            |

### Case 5.2: Midnight — first and last only
| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 00:00:00 | USER   | 12:45 call Poly       | call Poly | 12:45      |
| 2026-02-20 12:45:01 | SYSTEM | NONE                  |           |            |

### Case 6: End of month — fires next day crossing month boundary

| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-28 13:00:00 | USER   | 12:45 call Poly       | call Poly | 12:45      |
| 2026-02-28 13:00:00 | SYSTEM | 2026-03-01 12:45:00   |           |            |
| 2026-03-01 12:45:01 | SYSTEM | NONE                  |           |            |

### Case 6.2: End of month — first and last only
| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-28 13:00:00 | USER   | 12:45 call Poly       | call Poly | 12:45      |
| 2026-03-01 12:45:01 | SYSTEM | NONE                  |           |            |

### Case 7: Every 3 days — fires at next 15:30, does not deactivate

| Current Time        | Actor  | Input / Expected Next          | Message    | Normalized         |
|---------------------|--------|--------------------------------|------------|--------------------|
| 2026-02-20 10:00:00 | USER   | 15:30 every 3 days run backup  | run backup | 15:30 every 3 days |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 15:30:00            |            |                    |
| 2026-02-20 15:30:01 | SYSTEM | 2026-02-23 15:30:00            |            |                    |
| 2026-02-23 15:30:01 | SYSTEM | 2026-02-26 15:30:00            |            |                    |

### Case 7.2: Every 3 days — first and last only
| Current Time        | Actor  | Input / Expected Next          | Message    | Normalized         |
|---------------------|--------|--------------------------------|------------|--------------------|
| 2026-02-20 10:00:00 | USER   | 15:30 every 3 days run backup  | run backup | 15:30 every 3 days |
| 2026-02-23 15:30:01 | SYSTEM | 2026-02-26 15:30:00            |            |                    |

### Case 7.3: Every 7 min — first and several
| Current Time        | Actor  | Input / Expected Next          | Message    | Normalized            |
|---------------------|--------|--------------------------------|------------|-----------------------|
| 2026-02-20 19:31:00 | USER   | 19:30 every 7 min call Peter   | call Peter | 19:30 every 7 minutes |
| 2026-02-20 19:32:01 | SYSTEM | 2026-02-21 19:30:00            |            |                       |
| 2026-02-21 19:29:59 | SYSTEM | 2026-02-21 19:30:00            |            |                       |
| 2026-02-21 19:30:01 | SYSTEM | 2026-02-21 19:37:00            |            |                       |
| 2026-02-21 19:37:01 | SYSTEM | 2026-02-21 19:44:00            |            |                       |

### Case 7.4: Exact time and day of month + every 2 days (day-of-month has priority)
| Current Time        | Actor  | Input / Expected Next                            | Message  | Normalized                                |
|---------------------|--------|--------------------------------------------------|----------|-------------------------------------------|
| 2026-06-24 19:36:00 | USER   | 22:15 every 28 of the month every 2 day call Mal | call Mal | 22:15 each 28th day of the month every 2 days |
| 2026-06-24 19:36:01 | SYSTEM | 2026-06-28 22:15:00                              |          |                                           |
| 2026-06-28 22:15:01 | SYSTEM | 2026-06-30 22:15:00                              |          |                                           |
| 2026-06-30 22:15:01 | SYSTEM | 2026-07-02 22:15:00                              |          |                                           |
| 2026-07-28 22:14:01 | SYSTEM | 2026-07-28 22:15:00                              |          |                                           |
| 2026-07-28 22:15:01 | SYSTEM | 2026-07-30 22:15:00                              |          |                                           |

### Case 7.5: Day of month only — fires monthly on the 28th, no repetition
| Current Time        | Actor  | Input / Expected Next            | Message  | Normalized                    |
|---------------------|--------|----------------------------------|----------|-------------------------------|
| 2026-06-24 19:36:00 | USER   | 22:15 28th of the month call Mal | call Mal | 22:15 each 28th day of the month  |
| 2026-06-24 19:36:01 | SYSTEM | 2026-06-28 22:15:00              |          |                               |
| 2026-06-28 22:15:01 | SYSTEM | 2026-07-28 22:15:00              |          |                               |
| 2026-07-28 22:15:01 | SYSTEM | 2026-08-28 22:15:00              |          |                               |

### Case 7.6: Day of month with literal "day" — same as canonical form
| Current Time        | Actor  | Input / Expected Next                | Message  | Normalized                  |
|---------------------|--------|--------------------------------------|----------|-----------------------------|
| 2026-06-24 19:36:00 | USER   | 22:15 28th day of the month call Mal | call Mal | 22:15 each 28th day of the month |
| 2026-06-24 19:36:01 | SYSTEM | 2026-06-28 22:15:00                  |          |                             |
| 2026-06-28 22:15:01 | SYSTEM | 2026-07-28 22:15:00                  |          |                             |


### Case 8: Every year — fires at next 1:34, does not deactivate

| Current Time        | Actor  | Input / Expected Next       | Message     | Normalized       |
|---------------------|--------|-----------------------------|-------------|------------------|
| 2026-02-20 23:59:59 | USER   | 1:34 every year check lease | check lease | 01:34 every year |
| 2026-02-21 00:00:00 | SYSTEM | 2026-02-21 01:34:00         |             |                  |
| 2026-02-21 01:34:01 | SYSTEM | 2027-02-21 01:34:00         |             |                  |

### Case 8.2: Every year — first and last only
| Current Time        | Actor  | Input / Expected Next       | Message     | Normalized       |
|---------------------|--------|-----------------------------|-------------|------------------|
| 2026-02-20 23:59:59 | USER   | 1:34 every year check lease | check lease | 01:34 every year |
| 2026-02-21 01:34:01 | SYSTEM | 2027-02-21 01:34:00         |             |                  |

### Case 9: Every year - fires at next 1:34 next year, does not deactivate

| Current Time        | Actor  | Input / Expected Next          | Message        | Normalized       |
|---------------------|--------|--------------------------------|----------------|------------------|
| 2026-12-31 23:59:59 | USER   | 1:06 every year happy new year | happy new year | 01:06 every year |
| 2026-12-31 23:59:59 | SYSTEM | 2027-01-01 01:06:00            |                |                  |
| 2027-01-01 01:06:01 | SYSTEM | 2028-01-01 01:06:00            |                |                  |
| 2028-01-01 01:06:01 | SYSTEM | 2029-01-01 01:06:00            |                |                  |

### Case 9.2: Every year next year — first and last only
| Current Time        | Actor  | Input / Expected Next          | Message        | Normalized       |
|---------------------|--------|--------------------------------|----------------|------------------|
| 2026-12-31 23:59:59 | USER   | 1:06 every year happy new year | happy new year | 01:06 every year |
| 2028-01-01 01:06:01 | SYSTEM | 2029-01-01 01:06:00            |                |                  |

### Case 9.3: Birthday every year
| Current Time        | Actor  | Input / Expected Next          | Message        | Normalized       |
|---------------------|--------|--------------------------------|----------------|------------------|
| 2026-12-15 10:03:01 | USER   | 10:03 15.12 Poly's bday        | Poly's bday    | 10:03 15.12 yearly |
| 2026-12-15 10:03:01 | SYSTEM | 2027-12-15 10:03:00            |                |                  |
| 2027-12-15 10:03:01 | SYSTEM | 2028-12-15 10:03:00            |                |                  |
| 2028-12-15 10:03:01 | SYSTEM | 2029-12-15 10:03:00            |                |                  |

### Case 9.4: Birthday every year with optional "every year"
| Current Time        | Actor  | Input / Expected Next               | Message        | Normalized       |
|---------------------|--------|-------------------------------------|----------------|------------------|
| 2026-12-15 10:03:01 | USER   | 10:03 15.12 Every yeaR Poly's bday  | Poly's bday    | 10:03 15.12 yearly |
| 2026-12-15 10:03:01 | SYSTEM | 2027-12-15 10:03:00            |                |                  |
| 2027-12-15 10:03:01 | SYSTEM | 2028-12-15 10:03:00            |                |                  |
| 2028-12-15 10:03:01 | SYSTEM | 2029-12-15 10:03:00            |                |                  |

### Case 9.5: Birthday every with explicit year
| Current Time        | Actor  | Input / Expected Next                     | Message        | Normalized       |
|---------------------|--------|-------------------------------------------|----------------|------------------|
| 2026-12-15 10:03:01 | USER   | 10:03 15.12.2027 every year Poly's bday   | Poly's bday    | 10:03 15.12.2027 every year |
| 2026-12-15 10:03:01 | SYSTEM | 2027-12-15 10:03:00            |                |                  |
| 2027-12-15 10:03:01 | SYSTEM | 2028-12-15 10:03:00            |                |                  |
| 2028-12-15 10:03:01 | SYSTEM | 2029-12-15 10:03:00            |                |                  |

### Case 9.6: Birthday fires once
| Current Time        | Actor  | Input / Expected Next          | Message        | Normalized       |
|---------------------|--------|--------------------------------|----------------|------------------|
| 2026-12-15 10:03:01 | USER   | 10:03 15.12.2027 Poly's bday   | Poly's bday    | 10:03 15.12.2027 |
| 2026-12-15 10:03:01 | SYSTEM | 2027-12-15 10:03:00            |                |                  |
| 2027-12-15 10:03:01 | SYSTEM | NONE                           |                |                  |
| 2028-12-15 10:03:01 | SYSTEM | NONE                           |                |                  |

### Case 9.7: Birthday never fires
| Current Time        | Actor  | Input / Expected Next          | Message        | Normalized       |
|---------------------|--------|--------------------------------|----------------|------------------|
| 2026-12-15 10:03:01 | USER   | 10:03 15.12.2026 Poly's bday   | Poly's bday    | 10:03 15.12.2026 |
| 2026-12-15 10:03:01 | SYSTEM | NONE                           |                |                  |
| 2027-12-15 10:03:01 | SYSTEM | NONE                           |                |                  |
| 2028-12-15 10:03:01 | SYSTEM | NONE                           |                |                  |

### Case 10: every month - fires at next 20:00, does not deactivate

| Current Time        | Actor  | Input / Expected Next          | Message    | Normalized        |
|---------------------|--------|--------------------------------|------------|-------------------|
| 2026-02-20 23:59:59 | USER   | 20:00 every month run report   | run report | 20:00 every month |
| 2026-02-20 23:59:59 | SYSTEM | 2026-02-21 20:00:00            |            |                   |
| 2026-02-21 20:00:01 | SYSTEM | 2026-03-21 20:00:00            |            |                   |
| 2026-03-21 20:00:01 | SYSTEM | 2026-04-21 20:00:00            |            |                   |

### Case 10.2: Every month — first and last only
| Current Time        | Actor  | Input / Expected Next          | Message    | Normalized        |
|---------------------|--------|--------------------------------|------------|-------------------|
| 2026-02-20 23:59:59 | USER   | 20:00 every month run report   | run report | 20:00 every month |
| 2026-03-21 20:00:01 | SYSTEM | 2026-04-21 20:00:00            |            |                   |

### Case 11: One-shot with explicit date — fires once on the given date, then deactivates

| Current Time        | Actor  | Input / Expected Next            | Message         | Normalized       |
|---------------------|--------|----------------------------------|-----------------|------------------|
| 2026-02-21 09:00:00 | USER   | 11:26 12.10.2026 call the office | call the office | 11:26 12.10.2026 |
| 2026-02-21 09:00:00 | SYSTEM | 2026-10-12 11:26:00              |                 |                  |
| 2026-10-12 11:26:01 | SYSTEM | NONE                             |                 |                  |

### Case 11.2: One-shot with explicit date — first and last only
| Current Time        | Actor  | Input / Expected Next            | Message         | Normalized       |
|---------------------|--------|----------------------------------|-----------------|------------------|
| 2026-02-21 09:00:00 | USER   | 11:26 12.10.2026 call the office | call the office | 11:26 12.10.2026 |
| 2026-10-12 11:26:01 | SYSTEM | NONE                             |                 |                  |

### Case 12: Explicit date with repetition — fires on the given date, then repeats every 2 weeks

| Current Time        | Actor   | Input / Expected Next                    | Message   | Normalized                     |
|---------------------|---------|------------------------------------------|-----------|--------------------------------|
| 2026-02-21 09:00:00 | USER    | 11:26 12.10.2026 every 2 weeks sync team | sync team | 11:26 12.10.2026 every 2 weeks |
| 2026-02-21 09:00:00 | SYSTEM  | 2026-10-12 11:26:00                      |           |                                |
| 2026-10-12 11:26:01 | SYSTEM  | 2026-10-26 11:26:00                      |           |                                |
| 2026-10-26 11:26:01 | SYSTEM  | 2026-11-09 11:26:00                      |           |                                |

### Case 12.2: Explicit date with repetition — first and last only
| Current Time        | Actor   | Input / Expected Next                    | Message   | Normalized                     |
|---------------------|---------|------------------------------------------|-----------|--------------------------------|
| 2026-02-21 09:00:00 | USER    | 11:26 12.10.2026 every 2 weeks sync team | sync team | 11:26 12.10.2026 every 2 weeks |
| 2026-10-26 11:26:01 | SYSTEM  | 2026-11-09 11:26:00                      |           |                                |

### Case 13: Single weekday — created on that weekday, fires today then repeats weekly
| Current Time        | Actor  | Input / Expected Next   | Message     | Normalized |
|---------------------|--------|-------------------------|-------------|------------|
| 2026-02-20 10:00:00 | USER   | 10:30 fri release day   | release day | 10:30 Fri  |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 10:30:00     |             |            |
| 2026-02-20 10:30:01 | SYSTEM | 2026-02-27 10:30:00     |             |            |
| 2026-02-27 10:30:01 | SYSTEM | 2026-03-06 10:30:00     |             |            |

### Case 13.2: "Every friday" — the same as "friday"
| Current Time        | Actor  | Input / Expected Next        | Message     | Normalized |
|---------------------|--------|------------------------------|-------------|------------|
| 2026-02-20 10:00:00 | USER   | 10:30 Every  fri release day | release day | 10:30 Fri  |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 10:30:00          |             |            |
| 2026-02-20 10:30:01 | SYSTEM | 2026-02-27 10:30:00          |             |            |
| 2026-02-27 10:30:01 | SYSTEM | 2026-03-06 10:30:00          |             |            |

### Case 13.3: Single weekday — first and last only
| Current Time        | Actor  | Input / Expected Next   | Message     | Normalized |
|---------------------|--------|-------------------------|-------------|------------|
| 2026-02-20 10:00:00 | USER   | 10:30 fri release day   | release day | 10:30 Fri  |
| 2026-02-27 10:30:01 | SYSTEM | 2026-03-06 10:30:00     |             |            |

### Case 14: Single weekday — created mid-week, skips to next matching day then repeats weekly

| Current Time        | Actor  | Input / Expected Next      | Message     | Normalized |
|---------------------|--------|----------------------------|-------------|------------|
| 2026-02-23 09:00:00 | USER   | 10:30 FridaY release day   | release day | 10:30 Fri  |
| 2026-02-23 09:00:00 | SYSTEM | 2026-02-27 10:30:00        |             |            |
| 2026-02-27 10:30:01 | SYSTEM | 2026-03-06 10:30:00        |             |            |

### Case 14.2: Single weekday mid-week — first and last only
| Current Time        | Actor  | Input / Expected Next      | Message     | Normalized |
|---------------------|--------|----------------------------|-------------|------------|
| 2026-02-23 09:00:00 | USER   | 10:30 FridaY release day   | release day | 10:30 Fri  |
| 2026-02-27 10:30:01 | SYSTEM | 2026-03-06 10:30:00        |             |            |

### Case 15: Weekday range mon-fri — fires on each weekday, skips weekend

| Current Time        | Actor  | Input / Expected Next        | Message       | Normalized    |
|---------------------|--------|------------------------------|---------------|---------------|
| 2026-02-19 10:26:00 | USER   | 10:25 mon-fri Daily standup  | Daily standup | 10:25 Mon-Fri |
| 2026-02-19 10:25:01 | SYSTEM | 2026-02-20 10:25:00          |               |               |
| 2026-02-20 10:25:01 | SYSTEM | 2026-02-23 10:25:00          |               |               |
| 2026-02-23 10:25:01 | SYSTEM | 2026-02-24 10:25:00          |               |               |
| 2026-02-24 10:25:01 | SYSTEM | 2026-02-25 10:25:00          |               |               |
| 2026-02-25 10:25:01 | SYSTEM | 2026-02-26 10:25:00          |               |               |
| 2026-02-26 10:25:01 | SYSTEM | 2026-02-27 10:25:00          |               |               |
| 2026-02-27 10:25:01 | SYSTEM | 2026-03-02 10:25:00          |               |               |

### Case 15.2: Weekday range mon-fri — first and last only
| Current Time        | Actor  | Input / Expected Next        | Message       | Normalized    |
|---------------------|--------|------------------------------|---------------|---------------|
| 2026-02-19 10:26:00 | USER   | 10:25 mon-fri Daily standup  | Daily standup | 10:25 Mon-Fri |
| 2026-02-27 10:25:01 | SYSTEM | 2026-03-02 10:25:00          |               |               |

### Case 16: Explicit year + weekdays — date falls on an allowed weekdays, fires only in 2027

| Current Time        | Actor  | Input / Expected Next                | Message          | Normalized          |
|---------------------|--------|--------------------------------------|------------------|---------------------|
| 2026-02-21 09:00:00 | USER   | 13:25 2027 fri,sun new year standup  | new year standup | 13:25 2027 Fri,Sun  |
| 2026-02-21 09:00:00 | SYSTEM | 2027-01-01 13:25:00                  |                  |                     |
| 2026-12-31 23:59:59 | SYSTEM | 2027-01-01 13:25:00                  |                  |                     |
| 2027-01-01 13:25:01 | SYSTEM | 2027-01-03 13:25:00                  |                  |                     |
| 2027-01-03 13:25:01 | SYSTEM | 2027-01-08 13:25:00                  |                  |                     |
| 2027-12-31 09:00:01 | SYSTEM | 2027-12-31 13:25:00                  |                  |                     |
| 2027-12-31 13:25:01 | SYSTEM | NONE                                 |                  |                     |

### Case 16.2: Explicit year + weekdays — first and last only
| Current Time        | Actor  | Input / Expected Next                | Message          | Normalized          |
|---------------------|--------|--------------------------------------|------------------|---------------------|
| 2026-02-21 09:00:00 | USER   | 13:25 2027 fri,sun new year standup  | new year standup | 13:25 2027 Fri,Sun  |
| 2027-12-31 13:25:01 | SYSTEM | NONE                                 |                  |                     |

### Case 17: Bare hour — future today, fires at 08:00 same day

| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 07:00:00 | USER   | 8 call Alex           | call Alex | 08:00      |
| 2026-02-20 07:00:00 | SYSTEM | 2026-02-20 08:00:00   |           |            |
| 2026-02-20 08:00:01 | SYSTEM | NONE                  |           |            |

### Case 17.2: Bare hour future today — first and last only
| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 07:00:00 | USER   | 8 call Alex           | call Alex | 08:00      |
| 2026-02-20 08:00:01 | SYSTEM | NONE                  |           |            |

### Case 18: Bare hour — past today, fires at 08:00 next day

| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 09:00:00 | USER   | 8 call Alex           | call Alex | 08:00      |
| 2026-02-20 09:00:00 | SYSTEM | 2026-02-21 08:00:00   |           |            |
| 2026-02-21 08:00:01 | SYSTEM | NONE                  |           |            |

### Case 18.2: Bare hour past today — first and last only
| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 09:00:00 | USER   | 8 call Alex           | call Alex | 08:00      |
| 2026-02-21 08:00:01 | SYSTEM | NONE                  |           |            |

### Case 19: Bare hour — exact match is not strictly future, fires next day

| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 08:00:00 | USER   | 8 call Alex           | call Alex | 08:00      |
| 2026-02-20 08:00:00 | SYSTEM | 2026-02-21 08:00:00   |           |            |
| 2026-02-21 08:00:01 | SYSTEM | NONE                  |           |            |

### Case 19.2: Bare hour exact match — first and last only
| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 08:00:00 | USER   | 8 call Alex           | call Alex | 08:00      |
| 2026-02-21 08:00:01 | SYSTEM | NONE                  |           |            |

### Case 20: Bare hour 24 — treated as 00:00, fires at next midnight

| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 10:00:00 | USER   | 24 call Poly          | call Poly | 00:00      |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-21 00:00:00   |           |            |
| 2026-02-21 00:00:01 | SYSTEM | NONE                  |           |            |

### Case 20.2: Bare hour 24 — first and last only
| Current Time        | Actor  | Input / Expected Next | Message   | Normalized |
|---------------------|--------|-----------------------|-----------|------------|
| 2026-02-20 10:00:00 | USER   | 24 call Poly          | call Poly | 00:00      |
| 2026-02-21 00:00:01 | SYSTEM | NONE                  |           |            |

### Case 21: Bare hour 25 it's not a valid hour

| Current Time        | Actor  | Input / Expected Next | Message | Normalized |
|---------------------|--------|-----------------------|---------|------------|
| 2026-02-20 10:00:00 | USER   | 25 call Poly          |         |            |
| 2026-02-20 10:00:00 | SYSTEM | NONE                  |         |            |

### Case 22: Bare hour with repetition

| Current Time        | Actor  | Input / Expected Next     | Message   | Normalized          |
|---------------------|--------|---------------------------|-----------|---------------------|
| 2026-02-20 10:00:00 | USER   | 21 call Poly every 2 days | call Poly | 21:00 every 2 days  |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 21:00:00       |           |                     |
| 2026-02-20 21:00:01 | SYSTEM | 2026-02-22 21:00:00       |           |                     |
| 2026-02-22 21:00:01 | SYSTEM | 2026-02-24 21:00:00       |           |                     |

### Case 22.2: Bare hour with repetition — first and last only
| Current Time        | Actor  | Input / Expected Next     | Message   | Normalized          |
|---------------------|--------|---------------------------|-----------|---------------------|
| 2026-02-20 10:00:00 | USER   | 21 call Poly every 2 days | call Poly | 21:00 every 2 days  |
| 2026-02-22 21:00:01 | SYSTEM | 2026-02-24 21:00:00       |           |                     |

### Case 23: In-offset minutes — one-shot, fires 8 minutes after creation

| Current Time        | Actor  | Input / Expected Next | Message  | Normalized   |
|---------------------|--------|-----------------------|----------|--------------|
| 2026-02-20 10:00:00 | USER   | 8 min call her        | call her | in 8 minutes |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 10:08:00   |          |              |
| 2026-02-20 10:08:01 | SYSTEM | NONE                  |          |              |

### Case 23.2: In-offset minutes — first and last only
| Current Time        | Actor  | Input / Expected Next | Message  | Normalized   |
|---------------------|--------|-----------------------|----------|--------------|
| 2026-02-20 10:00:00 | USER   | 8 min call her        | call her | in 8 minutes |
| 2026-02-20 10:08:01 | SYSTEM | NONE                  |          |              |

### Case 24: In-offset hours — one-shot, fires 3 hours after creation

| Current Time        | Actor  | Input / Expected Next  | Message        | Normalized |
|---------------------|--------|------------------------|----------------|------------|
| 2026-02-20 10:00:00 | USER   | 3 hour check the oven  | check the oven | in 3 hours |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 13:00:00    |                |            |
| 2026-02-20 13:00:01 | SYSTEM | NONE                   |                |            |

### Case 24.2: In-offset hours — first and last only
| Current Time        | Actor  | Input / Expected Next  | Message        | Normalized |
|---------------------|--------|------------------------|----------------|------------|
| 2026-02-20 10:00:00 | USER   | 3 hour check the oven  | check the oven | in 3 hours |
| 2026-02-20 13:00:01 | SYSTEM | NONE                   |                |            |

### Case 25: In-offset minutes crossing midnight — fires next day

| Current Time        | Actor  | Input / Expected Next      | Message            | Normalized    |
|---------------------|--------|----------------------------|--------------------|---------------|
| 2026-02-20 23:55:00 | USER   | 10 min take the pizza out  | take the pizza out | in 10 minutes |
| 2026-02-20 23:55:00 | SYSTEM | 2026-02-21 00:05:00        |                    |               |
| 2026-02-21 00:05:01 | SYSTEM | NONE                       |                    |               |

### Case 25.2: In-offset minutes crossing midnight — first and last only
| Current Time        | Actor  | Input / Expected Next      | Message            | Normalized    |
|---------------------|--------|----------------------------|--------------------|---------------|
| 2026-02-20 23:55:00 | USER   | 10 min take the pizza out  | take the pizza out | in 10 minutes |
| 2026-02-21 00:05:01 | SYSTEM | NONE                       |                    |               |

### Case 26: In-offset minutes with hourly repetition

| Current Time        | Actor  | Input / Expected Next          | Message      | Normalized              |
|---------------------|--------|--------------------------------|--------------|-------------------------|
| 2026-02-20 10:00:00 | USER   | 8 min every hour check server  | check server | in 8 minutes every hour |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 10:08:00            |              |                         |
| 2026-02-20 10:08:01 | SYSTEM | 2026-02-20 11:08:00            |              |                         |
| 2026-02-20 11:08:01 | SYSTEM | 2026-02-20 12:08:00            |              |                         |

### Case 26.2: In-offset minutes with hourly repetition — first and last only
| Current Time        | Actor  | Input / Expected Next          | Message      | Normalized              |
|---------------------|--------|--------------------------------|--------------|-------------------------|
| 2026-02-20 10:00:00 | USER   | 8 min every hour check server  | check server | in 8 minutes every hour |
| 2026-02-20 11:08:01 | SYSTEM | 2026-02-20 12:08:00            |              |                         |

### Case 27: In-offset hours with biweekly repetition

| Current Time        | Actor   | Input / Expected Next              | Message     | Normalized                 |
|---------------------|---------|------------------------------------|-------------|----------------------------|
| 2026-02-20 10:00:00 | USER    | 20 hours every 2 weeks sync report | sync report | in 20 hours every 2 weeks  |
| 2026-02-20 10:00:00 | SYSTEM  | 2026-02-21 06:00:00                |             |                            |
| 2026-02-21 06:00:01 | SYSTEM  | 2026-03-07 06:00:00                |             |                            |
| 2026-03-07 06:00:01 | SYSTEM  | 2026-03-21 06:00:00                |             |                            |

### Case 27.2: In-offset hours with biweekly repetition — first and last only
| Current Time        | Actor   | Input / Expected Next              | Message     | Normalized                 |
|---------------------|---------|------------------------------------|-------------|----------------------------|
| 2026-02-20 10:00:00 | USER    | 20 hours every 2 weeks sync report | sync report | in 20 hours every 2 weeks  |
| 2026-03-07 06:00:01 | SYSTEM  | 2026-03-21 06:00:00                |             |                            |

### Case 28: In-offset minutes with daily repetition

| Current Time        | Actor  | Input / Expected Next           | Message       | Normalized               |
|---------------------|--------|---------------------------------|---------------|--------------------------|
| 2026-02-20 09:00:00 | USER   | 30 min every day morning water  | morning water | in 30 minutes every day  |
| 2026-02-20 09:00:00 | SYSTEM | 2026-02-20 09:30:00             |               |                          |
| 2026-02-20 09:30:01 | SYSTEM | 2026-02-21 09:30:00             |               |                          |
| 2026-02-21 09:30:01 | SYSTEM | 2026-02-22 09:30:00             |               |                          |

### Case 28.2: In-offset minutes with daily repetition — first and last only
| Current Time        | Actor  | Input / Expected Next           | Message       | Normalized               |
|---------------------|--------|---------------------------------|---------------|--------------------------|
| 2026-02-20 09:00:00 | USER   | 30 min every day morning water  | morning water | in 30 minutes every day  |
| 2026-02-21 09:30:01 | SYSTEM | 2026-02-22 09:30:00             |               |                          |

### Case 28.3: In-offset with leading "in" — same as without it, repeats every 2 hours

| Current Time        | Actor  | Input / Expected Next       | Message | Normalized                |
|---------------------|--------|-----------------------------|---------|---------------------------|
| 2026-02-20 09:00:00 | USER   | in 8 min every 2 hour test  | test    | in 8 minutes every 2 hours |
| 2026-02-20 09:00:00 | SYSTEM | 2026-02-20 09:08:00         |         |                           |
| 2026-02-20 09:08:01 | SYSTEM | 2026-02-20 11:08:00         |         |                           |
| 2026-02-20 11:08:01 | SYSTEM | 2026-02-20 13:08:00         |         |                           |

### Case 29: First Sunday of month — created mid-month, skips to next month's first Sunday

| Current Time        | Actor   | Input / Expected Next           | Message     | Normalized         |
|---------------------|---------|---------------------------------|-------------|--------------------|
| 2026-02-20 10:00:00 | USER    | 10:00 first sunday buy package  | buy package | 10:00 first Sunday |
| 2026-02-20 10:00:00 | SYSTEM  | 2026-03-01 10:00:00             |             |                    |
| 2026-03-01 10:00:01 | SYSTEM  | 2026-04-05 10:00:00             |             |                    |
| 2026-04-05 10:00:01 | SYSTEM  | 2026-05-03 10:00:00             |             |                    |

### Case 29.2: First Sunday of month mid-month — first and last only
| Current Time        | Actor   | Input / Expected Next           | Message     | Normalized         |
|---------------------|---------|---------------------------------|-------------|--------------------|
| 2026-02-20 10:00:00 | USER    | 10:00 first sunday buy package  | buy package | 10:00 first Sunday |
| 2026-04-05 10:00:01 | SYSTEM  | 2026-05-03 10:00:00             |             |                    |

### Case 30: First Sunday of month — created before first Sunday of current month, fires this month

| Current Time        | Actor  | Input / Expected Next           | Message     | Normalized         |
|---------------------|--------|---------------------------------|-------------|--------------------|
| 2026-03-01 09:00:00 | USER   | 10:00 first sunday buy package  | buy package | 10:00 first Sunday |
| 2026-03-01 09:00:00 | SYSTEM | 2026-03-01 10:00:00             |             |                    |
| 2026-03-01 10:00:01 | SYSTEM | 2026-04-05 10:00:00             |             |                    |

### Case 30.2: First Sunday of month this month — first and last only
| Current Time        | Actor  | Input / Expected Next           | Message     | Normalized         |
|---------------------|--------|---------------------------------|-------------|--------------------|
| 2026-03-01 09:00:00 | USER   | 10:00 first sunday buy package  | buy package | 10:00 first Sunday |
| 2026-03-01 10:00:01 | SYSTEM | 2026-04-05 10:00:00             |             |                    |

### Case 31: Last Monday of month — created before last Monday of current month, fires this month

| Current Time        | Actor  | Input / Expected Next          | Message      | Normalized        |
|---------------------|--------|--------------------------------|--------------|-------------------|
| 2026-02-20 08:00:00 | USER   | 9:30 last monday sell package  | sell package | 09:30 last Monday |
| 2026-02-20 08:00:00 | SYSTEM | 2026-02-23 09:30:00            |              |                   |
| 2026-02-23 09:30:01 | SYSTEM | 2026-03-30 09:30:00            |              |                   |
| 2026-03-30 09:30:01 | SYSTEM | 2026-04-27 09:30:00            |              |                   |

### Case 31.2: Last Monday of month — first and last only
| Current Time        | Actor  | Input / Expected Next          | Message      | Normalized        |
|---------------------|--------|--------------------------------|--------------|-------------------|
| 2026-02-20 08:00:00 | USER   | 9:30 last monday sell package  | sell package | 09:30 last Monday |
| 2026-03-30 09:30:01 | SYSTEM | 2026-04-27 09:30:00            |              |                   |

### Case 32: Last Saturday of month — created after last Saturday of current month, skips to next month

| Current Time        | Actor  | Input / Expected Next       | Message      | Normalized          |
|---------------------|--------|-----------------------------|--------------|---------------------|
| 2026-02-28 11:35:00 | USER   | 9:30 last sat sell package  | sell package | 09:30 last Saturday |
| 2026-02-28 11:35:00 | SYSTEM | 2026-03-28 09:30:00         |              |                     |
| 2026-03-28 09:30:01 | SYSTEM | 2026-04-25 09:30:00         |              |                     |

### Case 32.2: Last Saturday of month — first and last only
| Current Time        | Actor  | Input / Expected Next       | Message      | Normalized          |
|---------------------|--------|-----------------------------|--------------|---------------------|
| 2026-02-28 11:35:00 | USER   | 9:30 last sat sell package  | sell package | 09:30 last Saturday |
| 2026-03-28 09:30:01 | SYSTEM | 2026-04-25 09:30:00         |              |                     |

### Case 33: Last day of month — fires on last day of each month

| Current Time        | Actor  | Input / Expected Next                  | Message   | Normalized                  |
|---------------------|--------|----------------------------------------|-----------|-----------------------------|
| 2026-02-05 10:00:00 | USER   | 18:00 last day of the month pay bills  | pay bills | 18:00 last day of the month |
| 2026-02-05 10:00:00 | SYSTEM | 2026-02-28 18:00:00                    |           |                             |
| 2026-02-28 18:00:01 | SYSTEM | 2026-03-31 18:00:00                    |           |                             |
| 2026-03-31 18:00:01 | SYSTEM | 2026-04-30 18:00:00                    |           |                             |
| 2026-12-31 17:59:00 | SYSTEM | 2026-12-31 18:00:00                    |           |                             |
| 2026-12-31 18:00:01 | SYSTEM | 2027-01-31 18:00:00                    |           |                             |

### Case 33.2: Last day of month — first and last only
| Current Time        | Actor  | Input / Expected Next                  | Message   | Normalized                  |
|---------------------|--------|----------------------------------------|-----------|-----------------------------|
| 2026-02-05 10:00:00 | USER   | 18:00 last day of the month pay bills  | pay bills | 18:00 last day of the month |
| 2026-12-31 18:00:01 | SYSTEM | 2027-01-31 18:00:00                    |           |                             |

### Case 34: Last day of month — "of the month" is optional

| Current Time        | Actor  | Input / Expected Next     | Message   | Normalized                  |
|---------------------|--------|---------------------------|-----------|-----------------------------|
| 2026-02-05 10:00:00 | USER   | 18:00 last day pay bills  | pay bills | 18:00 last day of the month |
| 2026-02-05 10:00:00 | SYSTEM | 2026-02-28 18:00:00       |           |                             |
| 2026-02-28 18:00:01 | SYSTEM | 2026-03-31 18:00:00       |           |                             |

### Case 34.2: Last day of month optional — first and last only
| Current Time        | Actor  | Input / Expected Next     | Message   | Normalized                  |
|---------------------|--------|---------------------------|-----------|-----------------------------|
| 2026-02-05 10:00:00 | USER   | 18:00 last day pay bills  | pay bills | 18:00 last day of the month |
| 2026-02-28 18:00:01 | SYSTEM | 2026-03-31 18:00:00       |           |                             |

### Case 35: Last day of month — created on the last day itself (exact time not yet reached)

| Current Time        | Actor  | Input / Expected Next                  | Message   | Normalized                  |
|---------------------|--------|----------------------------------------|-----------|-----------------------------|
| 2026-02-28 17:00:00 | USER   | 18:00 last day of the month pay bills  | pay bills | 18:00 last day of the month |
| 2026-02-28 17:00:00 | SYSTEM | 2026-02-28 18:00:00                    |           |                             |
| 2026-02-28 18:00:01 | SYSTEM | 2026-03-31 18:00:00                    |           |                             |

### Case 35.2: Last day of month on last day — first and last only
| Current Time        | Actor  | Input / Expected Next                  | Message   | Normalized                  |
|---------------------|--------|----------------------------------------|-----------|-----------------------------|
| 2026-02-28 17:00:00 | USER   | 18:00 last day of the month pay bills  | pay bills | 18:00 last day of the month |
| 2026-02-28 18:00:01 | SYSTEM | 2026-03-31 18:00:00                    |           |                             |

### Case 36: Last day of month — created on the last day after the time, skips to next month

| Current Time        | Actor  | Input / Expected Next                  | Message   | Normalized                  |
|---------------------|--------|----------------------------------------|-----------|-----------------------------|
| 2026-02-28 19:00:00 | USER   | 18:00 last day of the month pay bills  | pay bills | 18:00 last day of the month |
| 2026-02-28 19:00:00 | SYSTEM | 2026-03-31 18:00:00                    |           |                             |
| 2026-03-31 18:00:01 | SYSTEM | 2026-04-30 18:00:00                    |           |                             |

### Case 36.2: Last day of month after time — first and last only
| Current Time        | Actor  | Input / Expected Next                  | Message   | Normalized                  |
|---------------------|--------|----------------------------------------|-----------|-----------------------------|
| 2026-02-28 19:00:00 | USER   | 18:00 last day of the month pay bills  | pay bills | 18:00 last day of the month |
| 2026-03-31 18:00:01 | SYSTEM | 2026-04-30 18:00:00                    |           |                             |

### Case 37: 3rd Friday of month — created before 3rd Friday of current month, fires this month

| Current Time        | Actor   | Input / Expected Next       | Message    | Normalized        |
|---------------------|---------|-----------------------------|------------|-------------------|
| 2026-02-16 10:00:00 | USER    | 17:00 3rd friday happy hour | happy hour | 17:00 third Friday|
| 2026-02-16 10:00:00 | SYSTEM  | 2026-02-20 17:00:00         |            |                   |
| 2026-02-20 17:00:01 | SYSTEM  | 2026-03-20 17:00:00         |            |                   |
| 2026-03-20 17:00:01 | SYSTEM  | 2026-04-17 17:00:00         |            |                   |

### Case 37.2: 3rd Friday of month before — first and last only
| Current Time        | Actor   | Input / Expected Next       | Message    | Normalized        |
|---------------------|---------|-----------------------------|------------|-------------------|
| 2026-02-16 10:00:00 | USER    | 17:00 3rd friday happy hour | happy hour | 17:00 third Friday|
| 2026-03-20 17:00:01 | SYSTEM  | 2026-04-17 17:00:00         |            |                   |

### Case 38: 3rd Friday of month — created after 3rd Friday of current month, skips to next month

| Current Time        | Actor   | Input / Expected Next       | Message    | Normalized        |
|---------------------|---------|-----------------------------|------------|-------------------|
| 2026-02-21 10:00:00 | USER    | 17:00 3rd friday happy hour | happy hour | 17:00 third Friday|
| 2026-02-21 10:00:00 | SYSTEM  | 2026-03-20 17:00:00         |            |                   |
| 2026-03-20 17:00:01 | SYSTEM  | 2026-04-17 17:00:00         |            |                   |
| 2026-04-17 17:00:01 | SYSTEM  | 2026-05-15 17:00:00         |            |                   |

### Case 38.2: 3rd Friday of month after — first and last only
| Current Time        | Actor   | Input / Expected Next       | Message    | Normalized        |
|---------------------|---------|-----------------------------|------------|-------------------|
| 2026-02-21 10:00:00 | USER    | 17:00 3rd friday happy hour | happy hour | 17:00 third Friday|
| 2026-04-17 17:00:01 | SYSTEM  | 2026-05-15 17:00:00         |            |                   |

### Case 39: 5th Friday of month - some months will be skipped, not all months have 5 Fridays

| Current Time        | Actor   | Input / Expected Next       | Message    | Normalized        |
|---------------------|---------|-----------------------------|------------|-------------------|
| 2026-02-16 10:00:00 | USER    | 17:00 5th friday happy hour | happy hour | 17:00 fifth Friday|
| 2026-02-16 10:00:00 | SYSTEM  | 2026-05-29 17:00:00         |            |                   |
| 2026-05-29 17:00:01 | SYSTEM  | 2026-07-31 17:00:00         |            |                   |
| 2026-07-31 17:00:01 | SYSTEM  | 2026-10-30 17:00:00         |            |                   |

### Case 39.2: 5th Friday of month — first and last only
| Current Time        | Actor   | Input / Expected Next       | Message    | Normalized        |
|---------------------|---------|-----------------------------|------------|-------------------|
| 2026-02-16 10:00:00 | USER    | 17:00 5th friday happy hour | happy hour | 17:00 fifth Friday|
| 2026-07-31 17:00:01 | SYSTEM  | 2026-10-30 17:00:00         |            |                   |

### Case 40: First Friday of month with repition. Repition must be canceled just before next first Friday

| Current Time        | Actor  | Input / Expected Next                       | Message    | Normalized                       |
|---------------------|--------|---------------------------------------------|------------|----------------------------------|
| 2026-03-01 09:00:00 | USER   | 10:00 first friday every 10 days buy ticket | buy ticket | 10:00 first Friday every 10 days |
| 2026-03-01 09:00:00 | SYSTEM | 2026-03-06 10:00:00                         |            |                                  |
| 2026-03-06 10:00:01 | SYSTEM | 2026-03-16 10:00:00                         |            |                                  |
| 2026-03-16 10:00:01 | SYSTEM | 2026-03-26 10:00:00                         |            |                                  |
| 2026-03-26 10:00:01 | SYSTEM | 2026-04-03 10:00:00                         |            |                                  |
| 2026-04-03 10:00:01 | SYSTEM | 2026-04-13 10:00:00                         |            |                                  |

### Case 40.2: First Friday with repetition — first and last only
| Current Time        | Actor  | Input / Expected Next                       | Message    | Normalized                       |
|---------------------|--------|---------------------------------------------|------------|----------------------------------|
| 2026-03-01 09:00:00 | USER   | 10:00 first friday every 10 days buy ticket | buy ticket | 10:00 first Friday every 10 days |
| 2026-04-03 10:00:01 | SYSTEM | 2026-04-05 10:00:00                         |            |                                  |

### Case 41: Single-digit minute — "10:6" means "10:06", fires once

| Current Time        | Actor  | Input / Expected Next | Message | Normalized |
|---------------------|--------|-----------------------|---------|------------|
| 2026-02-20 09:00:00 | USER   | 10:6 standup          | standup | 10:06      |
| 2026-02-20 09:00:00 | SYSTEM | 2026-02-20 10:06:00   |         |            |
| 2026-02-20 10:06:01 | SYSTEM | NONE                  |         |            |

### Case 42: Multiline message — line breaks preserved, fires once

| Current Time        | Actor  | Input / Expected Next       | Message            | Normalized |
|---------------------|--------|-----------------------------|--------------------|------------|
| 2026-02-20 09:00:00 | USER   | 12:45 buy milk\ncall mom    | buy milk\ncall mom | 12:45      |
| 2026-02-20 09:00:00 | SYSTEM | 2026-02-20 12:45:00         |                    |            |
| 2026-02-20 12:45:01 | SYSTEM | NONE                        |                    |            |
