-- Plugin management permissions.

INSERT OR IGNORE INTO permissions (id, name, description) VALUES
    ('perm_plugins_read', 'plugins.read', 'View installed plugins'),
    ('perm_plugins_write', 'plugins.write', 'Configure and toggle plugins'),
    ('perm_plugins_install', 'plugins.install', 'Install, upgrade and remove plugins');

-- Owner and admin hold every plugin permission.
INSERT OR IGNORE INTO role_permissions (role_id, permission_id)
    SELECT r.id, p.id FROM roles r CROSS JOIN permissions p
    WHERE r.id IN ('role_owner', 'role_admin') AND p.name LIKE 'plugins.%';
