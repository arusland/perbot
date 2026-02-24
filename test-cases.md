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

### Case 17: Bare hour — future today, fires at 08:00 same day

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 07:00:00 | USER   | 8 call Alex           |
| 2026-02-20 07:00:00 | SYSTEM | 2026-02-20 08:00:00   |
| 2026-02-20 08:00:01 | SYSTEM | NONE                  |

### Case 18: Bare hour — past today, fires at 08:00 next day

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 09:00:00 | USER   | 8 call Alex           |
| 2026-02-20 09:00:00 | SYSTEM | 2026-02-21 08:00:00   |
| 2026-02-21 08:00:01 | SYSTEM | NONE                  |

### Case 19: Bare hour — exact match is not strictly future, fires next day

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 08:00:00 | USER   | 8 call Alex           |
| 2026-02-20 08:00:00 | SYSTEM | 2026-02-21 08:00:00   |
| 2026-02-21 08:00:01 | SYSTEM | NONE                  |

### Case 20: Bare hour 24 — treated as 00:00, fires at next midnight

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 10:00:00 | USER   | 24 call Poly          |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-21 00:00:00   |
| 2026-02-21 00:00:01 | SYSTEM | NONE                  |

### Case 21: Bare hour 25 it's not a valid hour

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 10:00:00 | USER   | 25 call Poly          |
| 2026-02-20 10:00:00 | SYSTEM | NONE                  |

### Case 22: Bare hour with repetition

| Current Time        | Actor  | Input / Expected Next     |
|---------------------|--------|---------------------------|
| 2026-02-20 10:00:00 | USER   | 21 call Poly every 2 days |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 21:00:00       |
| 2026-02-20 21:00:01 | SYSTEM | 2026-02-22 21:00:00       |
| 2026-02-22 21:00:01 | SYSTEM | 2026-02-24 21:00:00       |

### Case 23: In-offset minutes — one-shot, fires 8 minutes after creation

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 10:00:00 | USER   | 8 min call her        |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 10:08:00   |
| 2026-02-20 10:08:01 | SYSTEM | NONE                  |

### Case 24: In-offset hours — one-shot, fires 3 hours after creation

| Current Time        | Actor  | Input / Expected Next      |
|---------------------|--------|----------------------------|
| 2026-02-20 10:00:00 | USER   | 3 hour check the oven     |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 13:00:00        |
| 2026-02-20 13:00:01 | SYSTEM | NONE                       |

### Case 25: In-offset minutes crossing midnight — fires next day

| Current Time        | Actor  | Input / Expected Next      |
|---------------------|--------|----------------------------|
| 2026-02-20 23:55:00 | USER   | 10 min take the pizza out  |
| 2026-02-20 23:55:00 | SYSTEM | 2026-02-21 00:05:00        |
| 2026-02-21 00:05:01 | SYSTEM | NONE                       |

### Case 26: In-offset minutes with hourly repetition

| Current Time        | Actor  | Input / Expected Next          |
|---------------------|--------|--------------------------------|
| 2026-02-20 10:00:00 | USER   | 8 min every hour check server  |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 10:08:00            |
| 2026-02-20 10:08:01 | SYSTEM | 2026-02-20 11:08:00            |
| 2026-02-20 11:08:01 | SYSTEM | 2026-02-20 12:08:00            |

### Case 27: In-offset hours with biweekly repetition

| Current Time        | Actor  | Input / Expected Next              |
|---------------------|---------|------------------------------------|
| 2026-02-20 10:00:00 | USER   | 20 hours every 2 weeks sync report |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-21 06:00:00                |
| 2026-02-21 06:00:01 | SYSTEM | 2026-03-07 06:00:00                |
| 2026-03-07 06:00:01 | SYSTEM | 2026-03-21 06:00:00                |

### Case 28: In-offset minutes with daily repetition

| Current Time        | Actor  | Input / Expected Next           |
|---------------------|--------|---------------------------------|
| 2026-02-20 09:00:00 | USER   | 30 min every day morning water  |
| 2026-02-20 09:00:00 | SYSTEM | 2026-02-20 09:30:00             |
| 2026-02-20 09:30:01 | SYSTEM | 2026-02-21 09:30:00             |
| 2026-02-21 09:30:01 | SYSTEM | 2026-02-22 09:30:00             |

### Case 29: First Sunday of month — created mid-month, skips to next month's first Sunday

| Current Time        | Actor  | Input / Expected Next              |
|---------------------|---------|------------------------------------|
| 2026-02-20 10:00:00 | USER   | 10:00 first sunday buy package     |
| 2026-02-20 10:00:00 | SYSTEM | 2026-03-01 10:00:00                |
| 2026-03-01 10:00:01 | SYSTEM | 2026-04-05 10:00:00                |
| 2026-04-05 10:00:01 | SYSTEM | 2026-05-03 10:00:00                |

### Case 30: First Sunday of month — created before first Sunday of current month, fires this month

| Current Time        | Actor  | Input / Expected Next              |
|---------------------|--------|------------------------------------|
| 2026-03-01 09:00:00 | USER   | 10:00 first sunday buy package     |
| 2026-03-01 09:00:00 | SYSTEM | 2026-03-01 10:00:00                |
| 2026-03-01 10:00:01 | SYSTEM | 2026-04-05 10:00:00                |

### Case 31: Last Monday of month — created before last Monday of current month, fires this month

| Current Time        | Actor  | Input / Expected Next              |
|---------------------|--------|------------------------------------|
| 2026-02-20 08:00:00 | USER   | 9:30 last monday sell package      |
| 2026-02-20 08:00:00 | SYSTEM | 2026-02-23 09:30:00                |
| 2026-02-23 09:30:01 | SYSTEM | 2026-03-30 09:30:00                |
| 2026-03-30 09:30:01 | SYSTEM | 2026-04-27 09:30:00                |

### Case 32: Last Saturday of month — created after last Saturday of current month, skips to next month

| Current Time        | Actor  | Input / Expected Next              |
|---------------------|--------|------------------------------------|
| 2026-02-28 11:35:00 | USER   | 9:30 last sat sell package         |
| 2026-02-28 11:35:00 | SYSTEM | 2026-03-28 09:30:00                |
| 2026-03-28 09:30:01 | SYSTEM | 2026-04-25 09:30:00                |

### Case 33: Last day of month — fires on last day of each month

| Current Time        | Actor  | Input / Expected Next                  |
|---------------------|--------|----------------------------------------|
| 2026-02-05 10:00:00 | USER   | 18:00 last day of the month pay bills  |
| 2026-02-05 10:00:00 | SYSTEM | 2026-02-28 18:00:00                    |
| 2026-02-28 18:00:01 | SYSTEM | 2026-03-31 18:00:00                    |
| 2026-03-31 18:00:01 | SYSTEM | 2026-04-30 18:00:00                    |
| 2026-12-31 17:59:00 | SYSTEM | 2026-12-31 18:00:00                    |
| 2026-12-31 18:00:01 | SYSTEM | 2027-01-31 18:00:00                    |


### Case 34: Last day of month — "of the month" is optional

| Current Time        | Actor  | Input / Expected Next          |
|---------------------|--------|--------------------------------|
| 2026-02-05 10:00:00 | USER   | 18:00 last day pay bills       |
| 2026-02-05 10:00:00 | SYSTEM | 2026-02-28 18:00:00            |
| 2026-02-28 18:00:01 | SYSTEM | 2026-03-31 18:00:00            |

### Case 35: Last day of month — created on the last day itself (exact time not yet reached)

| Current Time        | Actor  | Input / Expected Next                  |
|---------------------|--------|----------------------------------------|
| 2026-02-28 17:00:00 | USER   | 18:00 last day of the month pay bills  |
| 2026-02-28 17:00:00 | SYSTEM | 2026-02-28 18:00:00                    |
| 2026-02-28 18:00:01 | SYSTEM | 2026-03-31 18:00:00                    |

### Case 36: Last day of month — created on the last day after the time, skips to next month

| Current Time        | Actor  | Input / Expected Next                  |
|---------------------|--------|----------------------------------------|
| 2026-02-28 19:00:00 | USER   | 18:00 last day of the month pay bills  |
| 2026-02-28 19:00:00 | SYSTEM | 2026-03-31 18:00:00                    |
| 2026-03-31 18:00:01 | SYSTEM | 2026-04-30 18:00:00                    |
