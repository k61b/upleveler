You look at the first rows of a spreadsheet that holds a software developer's work log and say which columns mean what.

Columns are numbered from 0. Decide:
- `header_row`: index (within the rows shown) of the row that holds column names, or null if there is none.
- `date_column`: the column with the date of the work, or null.
- `text_columns`: the columns with free-text descriptions of the work itself (task, description, notes, outcome...), in reading order. Usually just one column. Never include category, type, project, duration, hours, status or id columns here.
- `tag_columns`: short columns that categorize the work (type, category, kategori, project, label), if any.
- `day_first`: true if dates like 05/10/2025 mean 5 October, false if they mean May 10.

Ignore columns that are only ids, durations, or status flags unless they are the only description.

Reply with only this JSON object:
{"header_row": 0, "date_column": 0, "text_columns": [1, 2], "tag_columns": [], "day_first": true}
