# Test Cases

## Description
* This document contains several tables with test cases
* First column contains current time in format YYYY-MM-DD HH:MM:SS
* Second column is actor's type: USER or SYSTEM
* When actor is USER, third column contains user's input in chat like "12:45 call Poly", test framework should parse it as EventInfo and than map it to StoredEvent (now it's current StoredEvent)
* When actor is SYSTEM, test framework call function storage::play_at with current StoredEvent and current time from first column. Now returned value is new current StoredEvent. And current event's next_datetime must be equals to the time in third column (format YYYY-MM-DD HH:MM:SS). If after call play_at new StoredEvent.active==false then third column must be NONE.

## Cases

### Case 1: Future today — event fires once

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 10:00:00 | USER   | 12:45 call Poly       |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 12:45:00   |
| 2026-02-20 12:45:01 | SYSTEM | NONE                  |

### Case 2: Past today — time already passed, fires the next day

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 14:00:00 | USER   | 12:45 call Poly       |
| 2026-02-20 14:00:00 | SYSTEM | 2026-02-21 12:45:00   |
| 2026-02-21 12:45:01 | SYSTEM | NONE                  |

### Case 3: Exact time — 12:45:00 equals now, not strictly future, fires next day

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 12:45:00 | USER   | 12:45 call Poly       |
| 2026-02-20 12:45:00 | SYSTEM | 2026-02-21 12:45:00   |
| 2026-02-21 12:45:01 | SYSTEM | NONE                  |

### Case 4: One second before — barely future, fires today

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 12:44:59 | USER   | 12:45 call Poly       |
| 2026-02-20 12:44:59 | SYSTEM | 2026-02-20 12:45:00   |
| 2026-02-20 12:45:01 | SYSTEM | NONE                  |

### Case 5: Midnight — fires at 12:45 the same day

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 00:00:00 | USER   | 12:45 call Poly       |
| 2026-02-20 00:00:00 | SYSTEM | 2026-02-20 12:45:00   |
| 2026-02-20 12:45:01 | SYSTEM | NONE                  |

### Case 6: End of month — fires next day crossing month boundary

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-28 13:00:00 | USER   | 12:45 call Poly       |
| 2026-02-28 13:00:00 | SYSTEM | 2026-03-01 12:45:00   |
| 2026-03-01 12:45:01 | SYSTEM | NONE                  |

### Case 7: Every 3 days — fires at next 15:30, does not deactivate

| Current Time        | Actor  | Input / Expected Next          |
|---------------------|--------|--------------------------------|
| 2026-02-20 10:00:00 | USER   | 15:30 every 3 days run backup  |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 15:30:00            |
| 2026-02-20 15:30:01 | SYSTEM | 2026-02-23 15:30:00            |
| 2026-02-23 15:30:01 | SYSTEM | 2026-02-26 15:30:00            |

### Case 8: Every year — fires at next 1:34, does not deactivate

| Current Time        | Actor  | Input / Expected Next       |
|---------------------|--------|-----------------------------|
| 2026-02-20 23:59:59 | USER   | 1:34 every year check lease |
| 2026-02-21 00:00:00 | SYSTEM | 2026-02-21 01:34:00         |
| 2026-02-21 01:34:01 | SYSTEM | 2027-02-21 01:34:00         |

### Case 9: Every year - fires at next 1:34 next year, does not deactivate

| Current Time        | Actor  | Input / Expected Next          |
|---------------------|--------|--------------------------------|
| 2026-12-31 23:59:59 | USER   | 1:06 every year happy new year |
| 2026-12-31 23:59:59 | SYSTEM | 2027-01-01 01:06:00            |
| 2027-01-01 01:06:01 | SYSTEM | 2028-01-01 01:06:00            |
| 2028-01-01 01:06:01 | SYSTEM | 2029-01-01 01:06:00            |

### Case 10: every month - fires at next 20:00, does not deactivate

| Current Time        | Actor  | Input / Expected Next          |
|---------------------|--------|--------------------------------|
| 2026-02-20 23:59:59 | USER   | 20:00 every month run report   |
| 2026-02-20 23:59:59 | SYSTEM | 2026-02-21 20:00:00            |
| 2026-02-21 20:00:01 | SYSTEM | 2026-03-21 20:00:00            |
| 2026-03-21 20:00:01 | SYSTEM | 2026-04-21 20:00:00            |

### Case 11: One-shot with explicit date — fires once on the given date, then deactivates

| Current Time        | Actor  | Input / Expected Next            |
|---------------------|--------|----------------------------------|
| 2026-02-21 09:00:00 | USER   | 11:26 12.10.2026 call the office |
| 2026-02-21 09:00:00 | SYSTEM | 2026-10-12 11:26:00              |
| 2026-10-12 11:26:01 | SYSTEM | NONE                             |

### Case 12: Explicit date with repetition — fires on the given date, then repeats every 2 weeks

| Current Time        | Actor  | Input / Expected Next                    |
|---------------------|---------|-----------------------------------------|
| 2026-02-21 09:00:00 | USER   | 11:26 12.10.2026 every 2 weeks sync team |
| 2026-02-21 09:00:00 | SYSTEM | 2026-10-12 11:26:00                      |
| 2026-10-12 11:26:01 | SYSTEM | 2026-10-26 11:26:00                      |
| 2026-10-26 11:26:01 | SYSTEM | 2026-11-09 11:26:00                      |

### Case 13: Single weekday — created on that weekday, fires today then repeats weekly

| Current Time        | Actor  | Input / Expected Next   |
|---------------------|--------|-------------------------|
| 2026-02-20 10:00:00 | USER   | 10:30 fri release day   |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 10:30:00     |
| 2026-02-20 10:30:01 | SYSTEM | 2026-02-27 10:30:00     |
| 2026-02-27 10:30:01 | SYSTEM | 2026-03-06 10:30:00     |

### Case 14: Single weekday — created mid-week, skips to next matching day then repeats weekly

| Current Time        | Actor  | Input / Expected Next   |
|---------------------|--------|-------------------------|
| 2026-02-23 09:00:00 | USER   | 10:30 FridaY release day   |
| 2026-02-23 09:00:00 | SYSTEM | 2026-02-27 10:30:00     |
| 2026-02-27 10:30:01 | SYSTEM | 2026-03-06 10:30:00     |

### Case 15: Weekday range mon-fri — fires on each weekday, skips weekend

| Current Time        | Actor  | Input / Expected Next        |
|---------------------|--------|------------------------------|
| 2026-02-19 10:26:00 | USER   | 10:25 mon-fri Daily standup  |
| 2026-02-19 10:25:01 | SYSTEM | 2026-02-20 10:25:00          |
| 2026-02-20 10:25:01 | SYSTEM | 2026-02-23 10:25:00          |
| 2026-02-23 10:25:01 | SYSTEM | 2026-02-24 10:25:00          |
| 2026-02-24 10:25:01 | SYSTEM | 2026-02-25 10:25:00          |
| 2026-02-25 10:25:01 | SYSTEM | 2026-02-26 10:25:00          |
| 2026-02-26 10:25:01 | SYSTEM | 2026-02-27 10:25:00          |
| 2026-02-27 10:25:01 | SYSTEM | 2026-03-02 10:25:00          |

### Case 16: Explicit year + weekdays — date falls on an allowed weekdays, fires only in 2027

| Current Time        | Actor  | Input / Expected Next                        |
|---------------------|--------|----------------------------------------------|
| 2026-02-21 09:00:00 | USER   | 13:25 2027 fri,sun new year standup          |
| 2026-02-21 09:00:00 | SYSTEM | 2027-01-01 13:25:00                          |
| 2026-12-31 23:59:59 | SYSTEM | 2027-01-01 13:25:00                          |
| 2027-01-01 13:25:01 | SYSTEM | 2027-01-03 13:25:00                          |
| 2027-01-03 13:25:01 | SYSTEM | 2027-01-08 13:25:00                          |
| 2027-12-31 09:00:01 | SYSTEM | 2027-12-31 13:25:00                          |
| 2027-12-31 13:25:01 | SYSTEM | NONE                                         |
