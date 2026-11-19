-- What the provider declared for the action when it was proposed (actions.md): its reach and its
-- reversibility, kept with the proposal so a run's why never depends on a plugin still being
-- installed. Proposals recorded before this column were all `linear.create_issue`, whose manifest
-- declares these two values.
ALTER TABLE action_proposal ADD COLUMN reach TEXT NOT NULL DEFAULT 'others_see_it'
    CHECK (reach IN ('only_me', 'my_things', 'others_see_it', 'public'));
ALTER TABLE action_proposal ADD COLUMN reversibility TEXT NOT NULL DEFAULT 'can_be_undone'
    CHECK (reversibility IN ('can_be_undone', 'can_be_compensated', 'cannot_be_undone'));
