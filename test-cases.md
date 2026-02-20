# Test Cases

## Description
* This document contains several tables with test cases
* First column contains current time in format YYYY-MM-DD HH:MM:SS
* Second column is actor's type: USER or SYSTEM
* When actor is USER, third column contains user's input in chat like "12:45 call Poly", test framework should parse it as ParsedEvent and than map it to StoredEvent (now it's current StoredEvent)
* When actor is SYSTEM, test framework call function storage::play_at with current StoredEvent and current time from first column. Now returned value is new current StoredEvent. And current event's next_datetime must be equals to the time in third column (format YYYY-MM-DD HH:MM:SS). If after call play_at new StoredEvent.active==false then third column must be NONE.

## Cases

### Case 1: Future today — event fires later the same day

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

### Case 3: Exact time — 12:45:00 equals now, not strictly future, fires next day

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 12:45:00 | USER   | 12:45 call Poly       |
| 2026-02-20 12:45:00 | SYSTEM | 2026-02-21 12:45:00   |

### Case 4: One second before — barely future, fires today

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 12:44:59 | USER   | 12:45 call Poly       |
| 2026-02-20 12:44:59 | SYSTEM | 2026-02-20 12:45:00   |

### Case 5: Midnight — fires at 12:45 the same day

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 00:00:00 | USER   | 12:45 call Poly       |
| 2026-02-20 00:00:00 | SYSTEM | 2026-02-20 12:45:00   |

### Case 6: End of month — fires next day crossing month boundary

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-28 13:00:00 | USER   | 12:45 call Poly       |
| 2026-02-28 13:00:00 | SYSTEM | 2026-03-01 12:45:00   |

### Case 7: Multi-step reschedule — fire at 12:45:01, next occurrence is tomorrow

| Current Time        | Actor  | Input / Expected Next |
|---------------------|--------|-----------------------|
| 2026-02-20 10:00:00 | USER   | 12:45 call Poly       |
| 2026-02-20 10:00:00 | SYSTEM | 2026-02-20 12:45:00   |
| 2026-02-20 12:45:01 | SYSTEM | 2026-02-21 12:45:00   |
