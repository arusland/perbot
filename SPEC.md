# Specification

- Try to parse time and date from a message.
- Parsed message must be represented as a struct (ParsedEvent) with fields for date and time, period, and repetition.
- Struct's fields:
  - date: Date
  - time: Time
  - period: Period
  - repetition: Repetition
  - message: String
- Time and date can be: 13:23, 5:24 PM, 1:23 26.11, 31.12.2027
- Time and date eventually calculated as future datetime. 

Ignore next features:
- When event is fired it can be repeated once in x hours/minutes/etc. Or specific can be added for that alert
- User can specify a period. Every 3 days, every Sunday, two days before/after xxx, last day of the month, Easter, in two days,
