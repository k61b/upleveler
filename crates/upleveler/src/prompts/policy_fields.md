What an Upleveler AI policy can say. Upleveler is a work-log app that runs on each employee's own computer; the security team puts the policy on every computer and Upleveler enforces it in code.

- name: who the policy is from, e.g. "Acme engineering: AI use".
- contact: who to ask about it (an e-mail address or a team).
- mode: which AI model Upleveler may use. "any": no limit. "local": only a model on the person's own computer. "approved": a model on the computer, or one of the company's approved endpoints. "off": no AI at all.
- endpoints: when mode is "approved", the base URLs of the approved endpoints, e.g. "https://llm.example.com/v1".
- no_training: true when the provider of that endpoint has agreed not to train on the data, false when it may.
- retention: how long that provider keeps the data, e.g. "none" or "30 days".
- agreement: a short note on the agreement with that provider.
- share_notes: whether notes about people (1:1 notes, feedback) may be sent to an endpoint that is not on the computer. false keeps them on the computer.
- mask_words: words never sent as written (the company's name, customers, project code names, internal host names). They are replaced with placeholders.
- sensitive: what happens to text that looks sensitive (ID, bank, card or phone numbers, pasted messages, health, beliefs, union membership, criminal records, salary). "warn": the person may choose to send it anyway. "keep": it always stays on the computer.
- private_tags: extra #tags that keep an entry on the computer (besides #private), e.g. "confidential".
- sensitive_words: extra words that make text count as sensitive, e.g. the code name of a confidential project.

Not settled yet: "" for text, null for true/false, [] for lists. Never write a placeholder such as "not settled" or "unknown".
