-- A call of delegate_task that 0.2.0 refused stored only the refusal, and the
-- runtime wrote its text for the model from the refusal's worker. The texts
-- move to the Workers plugin, and the core reads an executor's refusal from
-- the text in the part (ADR-0031, decision 6), so each part keeps the text
-- that 0.2.0 gave the model, word for word.
UPDATE runtime_parts
SET
  document = json_set(
    document,
    '$.task.output',
    CASE json_extract(document, '$.task.refusal.reason')
      WHEN 'worker_offline' THEN
        'Not run: the worker ' || json_extract(document, '$.task.refusal.worker')
        || ' is offline. Choose another worker, or tell the owner.'
      WHEN 'worker_unknown' THEN
        'Not run: there is no worker named ' || json_extract(document, '$.task.refusal.worker')
        || '.'
      WHEN 'worker_incompatible' THEN
        'Not run: the worker ' || json_extract(document, '$.task.refusal.worker')
        || ' runs a version of Mannie that this one does not work with. Tell the owner to update it.'
      ELSE
        'Not run: this conversation''s provider profile uses an address on this computer, which '
        || 'the worker ' || json_extract(document, '$.task.refusal.worker')
        || ' cannot reach. Tell the owner.'
    END
  )
WHERE
  kind = 'task'
  AND json_extract(document, '$.task.refusal.reason') IN (
    'worker_offline', 'worker_unknown', 'worker_incompatible', 'profile_not_reachable'
  )
  AND json_type(document, '$.task.refusal.worker') = 'text'
  AND coalesce(json_type(document, '$.task.output'), 'null') = 'null';
