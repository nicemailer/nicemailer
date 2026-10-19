-- ============================================================
-- NiceMailer : schéma SQLite du noyau Rust (version 1)
-- Dates : INTEGER, secondes Unix UTC. Booléens : 0/1.
-- Nécessite SQLite >= 3.38 (STRICT, unixepoch) avec FTS5.
-- Analyse et justification : modele de donnees.md
-- ============================================================

-- journal_mode = WAL est persistant dans le fichier de base.
-- foreign_keys, synchronous et temp_store sont propres à CHAQUE connexion :
-- le code Rust doit les ré-appliquer à l'ouverture de toute connexion
-- (sans foreign_keys = ON, les ON DELETE CASCADE ne s'exécutent pas).
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA synchronous = NORMAL;
PRAGMA temp_store = MEMORY;
PRAGMA user_version = 1;   -- migrations : comparer à la version attendue au démarrage

-- ------------------------------------------------------------
-- Comptes
-- ------------------------------------------------------------
CREATE TABLE account (
    id            INTEGER PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    display_name  TEXT,
    imap_host     TEXT NOT NULL,
    imap_port     INTEGER NOT NULL DEFAULT 993,
    imap_security TEXT NOT NULL DEFAULT 'tls'
                  CHECK (imap_security IN ('tls','starttls','none')),
    smtp_host     TEXT NOT NULL,
    smtp_port     INTEGER NOT NULL DEFAULT 465,
    smtp_security TEXT NOT NULL DEFAULT 'tls'
                  CHECK (smtp_security IN ('tls','starttls','none')),
    auth_kind     TEXT NOT NULL CHECK (auth_kind IN ('password','oauth2')),
    keychain_ref  TEXT NOT NULL,          -- référence Keychain, jamais le secret
    -- authserv-id de confiance des en-têtes Authentication-Results du serveur.
    -- NULL = aucun détecté : verdicts 'unknown' (voir authentification-expediteur.md).
    authserv_id   TEXT,
    -- Synchronisation de la configuration par le dossier nicemailer-sync du serveur
    -- (voir synchronisation-configuration.md). Désactivée par défaut.
    config_sync   INTEGER NOT NULL DEFAULT 0 CHECK (config_sync IN (0,1)),
    created_at    INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

-- ------------------------------------------------------------
-- Dossiers IMAP
-- ------------------------------------------------------------
CREATE TABLE folder (
    id             INTEGER PRIMARY KEY,
    account_id     INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    path           TEXT NOT NULL,         -- chemin IMAP complet (ex. "INBOX/Clients")
    delimiter      TEXT,
    -- Dossiers du workflow (voir dossiers.md). NULL = dossier serveur non géré.
    -- 'drafts' : brouillons, synchronisés avec le téléphone (is_draft = flag \Draft).
    -- 'config' : dossier nicemailer-sync de la configuration synchronisée, exclu de
    -- tout le reste (aucune ligne message, ni screening, ni recherche).
    -- Favoris et "tous les messages" ne sont pas des dossiers :
    -- is_flagged et une recherche.
    role           TEXT CHECK (role IN
                   ('inbox','todo','waiting','archive','sent','newsletter','paperwork',
                    'quarantine','deleted','junk','banned','drafts','config')),
    role_changed_at INTEGER,              -- dernière attribution du rôle (synchro de la config)
    uidvalidity    INTEGER,               -- si change : purger le dossier et resynchroniser
    uidnext        INTEGER,
    highestmodseq  INTEGER,               -- CONDSTORE/QRESYNC
    total_count    INTEGER NOT NULL DEFAULT 0,
    unread_count   INTEGER NOT NULL DEFAULT 0,
    subscribed     INTEGER NOT NULL DEFAULT 1 CHECK (subscribed IN (0,1)),
    last_sync_at   INTEGER,
    UNIQUE (account_id, path)
) STRICT;

-- ------------------------------------------------------------
-- Table des contacts : adresses (address) et domaines (domain), communs à tous
-- les comptes, connus de l'application.
-- Premier attribut : la décision de screening (voir dossiers.md). D'autres
-- attributs pourront s'ajouter ; une ligne sans décision (NULL) est valide.
-- Un expéditeur sans décision (adresse, puis domaine) va en Quarantine.
-- Priorité : décision de l'adresse > décision du domaine > Quarantine.
-- Une règle favorable n'est appliquée que si l'expéditeur est suffisamment
-- authentifié ; une règle de domaine favorable exige toujours 'pass'
-- (voir authentification-expediteur.md).
-- screening_target : jamais 'quarantine' (un mail sorti de Quarantine n'y retourne pas).
-- screening_target NULL avec screening_decided_at renseigné = décision retirée
-- (pierre tombale, voir synchronisation-configuration.md).
-- Adresses et domaines sont stockés en minuscules.
-- ------------------------------------------------------------
CREATE TABLE address (
    id          INTEGER PRIMARY KEY,
    addr        TEXT NOT NULL COLLATE NOCASE,   -- adresse complète
    name        TEXT,    -- nom d'affichage : renseigné à la première rencontre, jamais
                         -- écrasé ensuite par l'ingestion, modifiable par l'utilisateur
    screening_target     TEXT           -- NULL = pas de décision
                         CHECK (screening_target IN ('inbox','newsletter','paperwork','banned')),
    screening_decided_at INTEGER,
    -- Confiance au premier usage : 1 dès qu'un mail 'pass' a été reçu de cette
    -- adresse ; jamais remis à 0 par l'ingestion.
    auth_seen   INTEGER NOT NULL DEFAULT 0 CHECK (auth_seen IN (0,1)),
    -- auto : 'pass' exigé si auth_seen = 1 ; required : toujours 'pass' ;
    -- not_required : tout verdict accepté (outil d'envoi mal configuré).
    auth_policy TEXT NOT NULL DEFAULT 'auto'
                CHECK (auth_policy IN ('auto','required','not_required')),
    auth_policy_changed_at INTEGER,     -- NULL = jamais modifiée par l'utilisateur
    UNIQUE (addr)
) STRICT;

-- Pas de politique d'authentification : une règle de domaine favorable exige 'pass'.
CREATE TABLE domain (
    id          INTEGER PRIMARY KEY,
    domain      TEXT NOT NULL COLLATE NOCASE,   -- ex. "example.com", sans sous-domaine
    screening_target     TEXT           -- NULL = pas de décision
                         CHECK (screening_target IN ('inbox','newsletter','paperwork','banned')),
    screening_decided_at INTEGER,
    UNIQUE (domain)
) STRICT;

-- ------------------------------------------------------------
-- Conversations (JWZ). Une conversation peut couvrir plusieurs dossiers.
-- ------------------------------------------------------------
CREATE TABLE thread (
    id             INTEGER PRIMARY KEY,
    account_id     INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    subject        TEXT NOT NULL DEFAULT '',
    last_date      INTEGER NOT NULL DEFAULT 0,
    message_count  INTEGER NOT NULL DEFAULT 0,
    unread_count   INTEGER NOT NULL DEFAULT 0
) STRICT;

-- Vue matérialisée "conversations par dossier" : rend la liste de threads
-- d'un dossier en simple parcours d'index, sans agrégation à chaque scroll.
CREATE TABLE thread_folder (
    thread_id      INTEGER NOT NULL REFERENCES thread(id)  ON DELETE CASCADE,
    folder_id     INTEGER NOT NULL REFERENCES folder(id) ON DELETE CASCADE,
    last_date      INTEGER NOT NULL,
    message_count  INTEGER NOT NULL,
    unread_count   INTEGER NOT NULL,
    PRIMARY KEY (thread_id, folder_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_thread_folder_list
    ON thread_folder (folder_id, last_date DESC, thread_id DESC);

-- ------------------------------------------------------------
-- Messages : en-têtes seulement (table étroite = liste rapide).
-- Le corps vit dans message_body.
-- ------------------------------------------------------------
CREATE TABLE message (
    id               INTEGER PRIMARY KEY,
    account_id       INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    folder_id       INTEGER NOT NULL REFERENCES folder(id) ON DELETE CASCADE,
    -- Adresse du message sur le serveur, utilisée par la synchro et le spool,
    -- jamais exposée dans le contrat. L'identité stable est id.
    -- NULL : déplacement en cours, UID dans le dossier cible pas encore connu.
    uid              INTEGER,
    thread_id       INTEGER REFERENCES thread(id) ON DELETE SET NULL,

    message_id_hdr   TEXT,                -- en-tête Message-ID, sans chevrons
    in_reply_to      TEXT,
    subject          TEXT NOT NULL DEFAULT '',
    from_name        TEXT,                -- dénormalisés : affichés dans chaque ligne
    from_addr        TEXT,
    date_sent        INTEGER,             -- en-tête Date
    date_received    INTEGER NOT NULL,    -- INTERNALDATE : clé de tri fiable
    size             INTEGER,
    snippet          TEXT,                -- ~200 caractères pour l'aperçu

    is_seen          INTEGER NOT NULL DEFAULT 0 CHECK (is_seen     IN (0,1)),
    is_flagged       INTEGER NOT NULL DEFAULT 0 CHECK (is_flagged  IN (0,1)),
    is_answered      INTEGER NOT NULL DEFAULT 0 CHECK (is_answered IN (0,1)),
    is_draft         INTEGER NOT NULL DEFAULT 0 CHECK (is_draft    IN (0,1)),
    is_deleted       INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted  IN (0,1)),
    has_attachments  INTEGER NOT NULL DEFAULT 0 CHECK (has_attachments IN (0,1)),

    -- Authentification de l'expéditeur, calculée à la réception et jamais
    -- recalculée (voir authentification-expediteur.md).
    auth_result      TEXT NOT NULL DEFAULT 'unknown'
                     CHECK (auth_result IN ('pass','fail','none','unknown')),
    auth_source      TEXT CHECK (auth_source IN ('server','local')),  -- NULL : pas de source
    auth_suspect     INTEGER NOT NULL DEFAULT 0  -- règle favorable refusée faute d'authentification
                     CHECK (auth_suspect IN (0,1)),

    modseq           INTEGER,             -- CONDSTORE, pour détecter les changements de flags
    body_state       INTEGER NOT NULL DEFAULT 0
                     CHECK (body_state IN (0,1)),  -- 0 en-têtes seuls, 1 message complet

    UNIQUE (folder_id, uid)               -- synchro ; plusieurs NULL autorisés
) STRICT;

-- Liste d'un dossier avec pagination par curseur (clé : date, id).
-- SQLite ajoute le rowid (= id) à la fin de chaque index : le tiebreaker est
-- implicite. L'index est déclaré croissant et parcouru à l'envers pour
-- ORDER BY date_received DESC, id DESC ; un DESC sur date_received casserait
-- la correspondance avec le rowid, qui reste croissant.
CREATE INDEX idx_message_list
    ON message (folder_id, date_received);

-- Filtre "non lus" : index partiel, minuscule et très rapide.
CREATE INDEX idx_message_unread
    ON message (folder_id, date_received) WHERE is_seen = 0;

CREATE INDEX idx_message_flagged
    ON message (folder_id, date_received) WHERE is_flagged = 1;

-- Messages d'une conversation, dans l'ordre chronologique.
CREATE INDEX idx_message_thread
    ON message (thread_id, date_received, id);

-- Retrouver un message par Message-ID (threading, déduplication).
CREATE INDEX idx_message_msgid
    ON message (account_id, message_id_hdr);

-- ------------------------------------------------------------
-- Références (en-tête References) pour le threading JWZ
-- ------------------------------------------------------------
CREATE TABLE message_reference (
    message_id  INTEGER NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    ref_msgid   TEXT NOT NULL,
    PRIMARY KEY (message_id, position)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_message_reference_ref ON message_reference (ref_msgid);

-- ------------------------------------------------------------
-- Destinataires (To, Cc, Bcc, Reply-To), référencés dans la table address.
-- Le nom d'affichage est dans address.name.
-- L'expéditeur est dans message (from_name, from_addr).
-- ------------------------------------------------------------
CREATE TABLE message_address (
    message_id  INTEGER NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    role        TEXT NOT NULL CHECK (role IN ('to','cc','bcc','reply_to')),
    position    INTEGER NOT NULL,
    address_id  INTEGER NOT NULL REFERENCES address(id),
    PRIMARY KEY (message_id, role, position)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_message_address_address ON message_address (address_id);

-- ------------------------------------------------------------
-- Mots-clés IMAP d'un message (tags compris, voir la table tag)
-- ------------------------------------------------------------
CREATE TABLE message_keyword (
    message_id  INTEGER NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    keyword     TEXT NOT NULL,
    PRIMARY KEY (message_id, keyword)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_message_keyword_kw ON message_keyword (keyword, message_id);

-- ------------------------------------------------------------
-- Tags (étiquettes de l'utilisateur) = mots-clés IMAP côté serveur.
-- Un tag est un mot-clé IMAP auquel l'application donne un nom lisible.
-- Correspondance : message_keyword.keyword = tag.keyword (même compte).
-- Les mots-clés sans ligne dans tag ($Forwarded, $MDNSent...) restent de
-- simples mots-clés techniques, non affichés comme tags.
-- Un mot-clé IMAP est un atome ASCII (ni espace, ni parenthèses, ni accents) :
-- `name` est le nom affiché, `keyword` sa forme encodée envoyée au serveur.
-- ------------------------------------------------------------
CREATE TABLE tag (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,                  -- nom affiché, libre
    keyword     TEXT NOT NULL COLLATE NOCASE,   -- mot-clé IMAP (insensible à la casse)
    color       TEXT,
    changed_at  INTEGER NOT NULL DEFAULT (unixepoch()),  -- synchro de la config
    deleted_at  INTEGER,                -- pierre tombale : tag supprimé, masqué partout
    UNIQUE (account_id, name),
    UNIQUE (account_id, keyword)
) STRICT;

-- ------------------------------------------------------------
-- Corps : interprétation du message brut (seul le brut sur disque est immuable).
-- Écrit à la réception du message complet, puis réécrit par reindex quand
-- l'analyseur ou l'assainissement évoluent. Ce n'est pas un cache : il n'est
-- jamais évincé, seule la suppression du message l'efface (cascade).
-- ------------------------------------------------------------
CREATE TABLE message_body (
    message_id      INTEGER PRIMARY KEY REFERENCES message(id) ON DELETE CASCADE,
    text_plain      TEXT,
    html_sanitized  TEXT,                 -- déjà passé par ammonia, images distantes retirées
    has_remote_content INTEGER NOT NULL DEFAULT 0 CHECK (has_remote_content IN (0,1)),
    parser_version  INTEGER NOT NULL,     -- version de l'analyseur et de l'assainissement ;
                                          -- reindex cible les lignes de version inférieure
    raw_path        TEXT,                 -- .eml brut sur disque, si conservé
    fetched_at      INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX idx_message_body_parser ON message_body (parser_version);

-- ------------------------------------------------------------
-- Pièces jointes (métadonnées ; les fichiers vivent sur disque)
-- ------------------------------------------------------------
CREATE TABLE attachment (
    id           INTEGER PRIMARY KEY,
    message_id   INTEGER NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    part_id      TEXT NOT NULL,           -- numéro de partie IMAP ("1.2")
    filename     TEXT,
    mime_type    TEXT NOT NULL,
    size         INTEGER,
    content_id   TEXT,                    -- pour les images inline (cid:)
    is_inline    INTEGER NOT NULL DEFAULT 0 CHECK (is_inline IN (0,1)),
    disk_path    TEXT,                    -- fichier extrait du brut, transmis aux clients
                                          -- par chemin ; NULL si non extrait ou purgé
    UNIQUE (message_id, part_id)
) STRICT;

CREATE INDEX idx_attachment_cid ON attachment (message_id, content_id);

-- ------------------------------------------------------------
-- Recherche plein texte. rowid = message.id.
-- remove_diacritics 2 : "eleve" trouve "élève".
-- ------------------------------------------------------------
CREATE VIRTUAL TABLE message_fts USING fts5 (
    subject, sender, recipients, body,
    tokenize = 'unicode61 remove_diacritics 2',
    prefix   = '2 3'
);

-- À l'insertion d'un message : indexer l'objet et l'expéditeur tout de suite
-- (corps vide), pour que la recherche marche dès la synchro des en-têtes.
CREATE TRIGGER message_fts_ai AFTER INSERT ON message BEGIN
    INSERT INTO message_fts (rowid, subject, sender, recipients, body)
    VALUES (new.id, new.subject,
            coalesce(new.from_name,'') || ' ' || coalesce(new.from_addr,''), '', '');
END;

CREATE TRIGGER message_fts_au AFTER UPDATE OF subject, from_name, from_addr ON message BEGIN
    UPDATE message_fts
       SET subject = new.subject,
           sender  = coalesce(new.from_name,'') || ' ' || coalesce(new.from_addr,'')
     WHERE rowid = new.id;
END;

CREATE TRIGGER message_fts_ad AFTER DELETE ON message BEGIN
    DELETE FROM message_fts WHERE rowid = old.id;
END;

-- Quand le corps arrive : l'ajouter à l'index.
CREATE TRIGGER message_body_fts_ai AFTER INSERT ON message_body BEGIN
    UPDATE message_fts SET body = coalesce(new.text_plain, '') WHERE rowid = new.message_id;
END;

-- Quand le corps est réinterprété (reindex) : mettre l'index à jour.
-- reindex procède par UPDATE, jamais par INSERT OR REPLACE (qui supprimerait
-- puis réinsérerait la ligne sans passer par ce trigger).
CREATE TRIGGER message_body_fts_au AFTER UPDATE OF text_plain ON message_body BEGIN
    UPDATE message_fts SET body = coalesce(new.text_plain, '') WHERE rowid = new.message_id;
END;

-- ------------------------------------------------------------
-- Spool : file d'actions locales à rejouer vers le serveur (hors-ligne).
-- L'action est déjà appliquée localement à l'insertion (affichage immédiat).
-- Erreur temporaire : reste 'queued', run_after repoussé.
-- Erreur définitive : 'reverted', l'état local est restauré depuis undo.
-- 'cancelled' : annulée sans exécution (fusion, ou annulation en série).
-- Voir modele de donnees.md, section 6.
-- ------------------------------------------------------------
CREATE TABLE pending_action (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN
                ('set_flags','move','copy','delete','append_draft','send','push_config')),
    -- push_config : écriture de la configuration synchronisée, sans message ;
    -- contenu calculé à l'exécution (voir synchronisation-configuration.md).
    message_id  INTEGER REFERENCES message(id) ON DELETE SET NULL,
    payload     TEXT NOT NULL,            -- JSON : move = {from_folder_id, from_uid, to_folder_id}
    undo        TEXT,                     -- JSON : état local à restaurer en cas d'échec définitif
    state       TEXT NOT NULL DEFAULT 'queued'
                CHECK (state IN ('queued','running','done','reverted','cancelled')),
    attempts    INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT,
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    run_after   INTEGER NOT NULL DEFAULT 0  -- backoff
) STRICT;

CREATE INDEX idx_pending_action_queue
    ON pending_action (account_id, state, run_after, id);

-- Actions en attente sur un message : fusion, exécution en série par message,
-- et consultation par la synchro (UID à ignorer, lignes à rattacher).
CREATE INDEX idx_pending_action_message
    ON pending_action (message_id, id) WHERE state IN ('queued','running');

-- ------------------------------------------------------------
-- Requêtes de référence (pour la couche Rust)
-- ------------------------------------------------------------
-- Page suivante d'un dossier (pagination par curseur, pas par OFFSET) :
--   SELECT id, uid, subject, from_name, from_addr, date_received,
--          snippet, is_seen, is_flagged, has_attachments
--     FROM message
--    WHERE folder_id = :mb
--      AND (date_received, id) < (:cursor_date, :cursor_id)
--    ORDER BY date_received DESC, id DESC
--    LIMIT 50;
--
-- Recherche plein texte, triée par pertinence :
--   SELECT m.id, m.subject, snippet(message_fts, 3, '<b>', '</b>', '…', 12)
--     FROM message_fts
--     JOIN message m ON m.id = message_fts.rowid
--    WHERE message_fts MATCH :q          -- ex. 'factur*' ou 'sender:dupont AND devis'
--    ORDER BY rank
--    LIMIT 50;
--
-- Liste de conversations d'un dossier :
--   SELECT t.id, t.subject, tm.last_date, tm.message_count, tm.unread_count
--     FROM thread_folder tm
--     JOIN thread t ON t.id = tm.thread_id
--    WHERE tm.folder_id = :mb
--      AND (tm.last_date, tm.thread_id) < (:cursor_date, :cursor_id)
--    ORDER BY tm.last_date DESC, tm.thread_id DESC
--    LIMIT 50;
