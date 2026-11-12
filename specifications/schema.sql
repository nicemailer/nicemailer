-- ============================================================
-- NiceMailer : schéma SQLite du noyau Rust (version 4)
-- Dates : INTEGER, secondes Unix UTC. Booléens : 0/1.
-- Nécessite SQLite >= 3.38 (STRICT, unixepoch) avec FTS5.
-- Analyse et justification : modele de donnees.md
-- Toutes les colonnes de date sont préfixées par date_.
--
-- Traçabilité (voir tracabilite.md) : les entités (account, folder, address,
-- domain, tag, message) portent date_created, date_modified, created_by et
-- modified_by. date_modified ne suit que les attributs visibles de
-- l'utilisateur, jamais les colonnes techniques. Les sources sont renseignées
-- par le noyau, d'après l'identité vérifiée du client, jamais déclarées par lui :
--   core        noyau (réception, screening automatique)
--   app         application SwiftUI
--   cli         CLI
--   tui         TUI
--   web         serveur web
--   mcp         agent MCP
--   config_sync synchronisation de la configuration entre Mac
--   imap        changement vu sur le serveur (téléphone, webmail, autre Mac)
-- Les tables de liaison et les agrégats n'en portent pas. L'historique
-- complet est dans audit_log.
-- ============================================================

-- journal_mode = WAL est persistant dans le fichier de base.
-- foreign_keys, synchronous et temp_store sont propres à CHAQUE connexion :
-- le code Rust doit les ré-appliquer à l'ouverture de toute connexion
-- (sans foreign_keys = ON, les ON DELETE CASCADE ne s'exécutent pas).
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA synchronous = NORMAL;
PRAGMA temp_store = MEMORY;
-- Une base existante est mise à jour par les migrations du noyau
-- (crates/nm-store/src/migrations.rs) ; ce fichier crée directement la
-- dernière version.
PRAGMA user_version = 11;   -- migrations : comparer à la version attendue au démarrage

-- ------------------------------------------------------------
-- Comptes : état seulement. Les paramètres (adresse, serveurs, identifiants,
-- identités, config_sync) sont dans accounts.toml, table [accounts.<config_key>] ;
-- les secrets dans le Keychain (voir configuration.md).
-- Un compte absent du fichier est désactivé, jamais supprimé par ce seul fait.
-- Exception : un compte d'archives (kind = 'archive') n'a ni serveur IMAP ni
-- SMTP et n'est pas dans accounts.toml. Il est créé par un import mbox
-- (import-initial.md, section 9) ; config_key est le nom donné à l'import et
-- address l'adresse de la boîte archivée.
-- ------------------------------------------------------------
CREATE TABLE account (
    id            INTEGER PRIMARY KEY,
    config_key    TEXT NOT NULL UNIQUE,   -- clé du compte dans accounts.toml, ou nom du compte d'archives
    kind          TEXT NOT NULL DEFAULT 'imap' CHECK (kind IN ('imap','archive')),
    -- Adresse de la boîte, pour un compte d'archives seulement (en minuscules) ;
    -- NULL pour un compte IMAP, dont l'adresse est dans accounts.toml.
    address       TEXT,
    -- authserv-id de confiance des en-têtes Authentication-Results, détecté.
    -- NULL = aucun détecté : verdicts 'unknown' (voir authentification-expediteur.md).
    -- La clé authserv_id du compte dans accounts.toml, si présente, le remplace.
    authserv_id   TEXT,
    date_created  INTEGER NOT NULL DEFAULT (unixepoch()),
    date_modified INTEGER NOT NULL DEFAULT (unixepoch()),
    created_by    TEXT NOT NULL CHECK (created_by  IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    modified_by   TEXT NOT NULL CHECK (modified_by IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    -- (version 4) Date à laquelle la clé du compte a disparu d'accounts.toml :
    -- le compte n'est plus synchronisé, ses données restent. NULL = actif.
    -- Remis à NULL quand la clé revient (configuration.md, section 6).
    -- Toujours NULL pour un compte d'archives, absent du fichier par nature.
    disabled_at   INTEGER,
    -- (version 4) Rang d'affichage du compte : sélecteur de compte (@), barre
    -- latérale, vue unifiée. Croissant ; à égalité, le plus ancien (id) d'abord.
    -- Un nouveau compte se place après les autres (max + 1). État de l'interface
    -- modifié par set_account_order, donc en base et pas dans accounts.toml.
    position      INTEGER NOT NULL DEFAULT 0
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
    date_role_modified INTEGER,           -- dernière attribution du rôle (synchro de la config)
    uidvalidity    INTEGER,               -- si change : purger le dossier et resynchroniser
    uidnext        INTEGER,
    highestmodseq  INTEGER,               -- CONDSTORE/QRESYNC
    total_count    INTEGER NOT NULL DEFAULT 0,
    unread_count   INTEGER NOT NULL DEFAULT 0,
    subscribed     INTEGER NOT NULL DEFAULT 1 CHECK (subscribed IN (0,1)),
    date_last_sync INTEGER,
    -- Inbox : UID à partir duquel les mails sont triés ; les UID inférieurs ont
    -- été importés sans screening. NULL = compte en création, screening suspendu
    -- (voir import-initial.md).
    uid_screening_start INTEGER,
    -- created_by : 'imap' (dossier trouvé sur le serveur) ou le client qui l'a fait créer.
    -- date_modified : changement de rôle ou d'abonnement, pas les compteurs ni la synchro.
    date_created   INTEGER NOT NULL DEFAULT (unixepoch()),
    date_modified  INTEGER NOT NULL DEFAULT (unixepoch()),
    created_by     TEXT NOT NULL CHECK (created_by  IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    modified_by    TEXT NOT NULL CHECK (modified_by IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
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
-- screening_target NULL avec date_screening_modified renseigné = décision retirée
-- (pierre tombale, voir synchronisation-configuration.md).
-- Adresses et domaines sont stockés en minuscules.
-- ------------------------------------------------------------
CREATE TABLE address (
    id          INTEGER PRIMARY KEY,
    addr        TEXT NOT NULL COLLATE NOCASE,   -- adresse complète
    name        TEXT,    -- nom d'affichage : renseigné à la première rencontre qui en
                         -- porte un (l'ingestion complète un nom vide), jamais écrasé
                         -- ensuite par l'ingestion, modifiable par l'utilisateur
    screening_target     TEXT           -- NULL = pas de décision
                         CHECK (screening_target IN ('inbox','newsletter','paperwork','banned')),
    date_screening_modified INTEGER,
    -- Confiance au premier usage : 1 dès qu'un mail 'pass' a été reçu de cette
    -- adresse ; jamais remis à 0 par l'ingestion.
    auth_seen   INTEGER NOT NULL DEFAULT 0 CHECK (auth_seen IN (0,1)),
    -- auto : 'pass' exigé si auth_seen = 1 ; required : toujours 'pass' ;
    -- not_required : tout verdict accepté (outil d'envoi mal configuré).
    auth_policy TEXT NOT NULL DEFAULT 'auto'
                CHECK (auth_policy IN ('auto','required','not_required')),
    date_auth_policy_modified INTEGER,  -- NULL = jamais modifiée par l'utilisateur
    -- Découverte par un import ; NULL = réception normale. Jamais effacé.
    imported_from TEXT CHECK (imported_from IN ('imap_initial','mbox')),
    -- Affichage préféré des mails de cette adresse : 'text' ou 'html' ; NULL =
    -- choix par défaut du thème (ui.md). Préférence d'affichage locale : ni
    -- synchronisée, ni journalisée, sans effet sur date_modified. Le mode
    -- « images » est 'html' avec remote_images = 1 (en fin de table).
    body_format TEXT CHECK (body_format IN ('text','html')),
    -- Désabonnement en un clic réussi depuis l'app (ui.md, « Désabonnement ») ;
    -- NULL = jamais. Sans effet sur la décision de screening.
    unsubscribed_at INTEGER,
    -- date_modified : nom, décision de screening ou auth_policy ; pas auth_seen.
    date_created INTEGER NOT NULL DEFAULT (unixepoch()),
    date_modified INTEGER NOT NULL DEFAULT (unixepoch()),
    created_by  TEXT NOT NULL CHECK (created_by  IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    modified_by TEXT NOT NULL CHECK (modified_by IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    -- Version 3. 1 : les images distantes des mails de cette adresse sont
    -- chargées (mode « images », ui.md, « Images distantes ») ; seulement avec
    -- body_format = 'html'. Même statut que body_format : local, non journalisé.
    remote_images INTEGER NOT NULL DEFAULT 0 CHECK (remote_images IN (0,1)),
    UNIQUE (addr)
) STRICT;

-- Pas de politique d'authentification : une règle de domaine favorable exige 'pass'.
CREATE TABLE domain (
    id          INTEGER PRIMARY KEY,
    domain      TEXT NOT NULL COLLATE NOCASE,   -- ex. "example.com", sans sous-domaine
    screening_target     TEXT           -- NULL = pas de décision
                         CHECK (screening_target IN ('inbox','newsletter','paperwork','banned')),
    date_screening_modified INTEGER,
    date_created INTEGER NOT NULL DEFAULT (unixepoch()),
    date_modified INTEGER NOT NULL DEFAULT (unixepoch()),
    created_by  TEXT NOT NULL CHECK (created_by  IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    modified_by TEXT NOT NULL CHECK (modified_by IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    UNIQUE (domain)
) STRICT;

-- ------------------------------------------------------------
-- Groupes de contacts (voir groupes.md) : ensembles nommés d'adresses
-- (VIP, famille, équipe, club…), communs à tous les comptes comme address.
-- Une adresse peut appartenir à plusieurs groupes. Les groupes n'ont pour
-- l'instant aucun effet sur le screening ni sur le classement.
-- Nommée contact_group car GROUP est un mot réservé de SQL.
-- Pierres tombales pour la synchronisation entre Mac
-- (synchronisation-configuration.md) : un groupe supprimé garde sa ligne avec
-- date_deleted ; un membre retiré garde sa ligne avec date_removed.
-- ------------------------------------------------------------
CREATE TABLE contact_group (
    id           INTEGER PRIMARY KEY,
    -- Identifiant stable entre Mac, tiré au hasard à la création ("g_" + 16
    -- caractères hexadécimaux) : l'id local diffère d'un Mac à l'autre, et le
    -- nom peut changer.
    sync_key     TEXT NOT NULL UNIQUE,
    name         TEXT NOT NULL,             -- nom affiché, libre
    color        TEXT,                      -- ex. "#f59e0b" ; NULL = couleur par défaut
    position     INTEGER NOT NULL DEFAULT 0,  -- ordre d'affichage, croissant
    date_deleted INTEGER,                   -- pierre tombale : groupe supprimé, masqué partout
    -- date_modified : nom, couleur, position, suppression ; sert aussi
    -- d'horodatage d'entrée pour la synchronisation.
    date_created  INTEGER NOT NULL DEFAULT (unixepoch()),
    date_modified INTEGER NOT NULL DEFAULT (unixepoch()),
    created_by  TEXT NOT NULL CHECK (created_by  IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    modified_by TEXT NOT NULL CHECK (modified_by IN ('core','app','cli','tui','web','mcp','config_sync','imap'))
) STRICT;

-- Deux groupes actifs ne peuvent pas porter le même nom (sans casse) ; un nom
-- d'un groupe supprimé peut être repris (la ligne supprimée est alors réutilisée).
CREATE UNIQUE INDEX idx_contact_group_name
    ON contact_group (name COLLATE NOCASE) WHERE date_deleted IS NULL;

CREATE TABLE contact_group_member (
    group_id     INTEGER NOT NULL REFERENCES contact_group(id) ON DELETE CASCADE,
    address_id   INTEGER NOT NULL REFERENCES address(id) ON DELETE CASCADE,
    date_removed INTEGER,                   -- pierre tombale : adresse retirée du groupe
    -- date_modified : ajout ou retrait ; horodatage d'entrée pour la synchronisation.
    date_created  INTEGER NOT NULL DEFAULT (unixepoch()),
    date_modified INTEGER NOT NULL DEFAULT (unixepoch()),
    created_by  TEXT NOT NULL CHECK (created_by  IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    modified_by TEXT NOT NULL CHECK (modified_by IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    PRIMARY KEY (group_id, address_id)
) STRICT, WITHOUT ROWID;

-- Groupes d'une adresse (fiche du contact, badges dans la liste).
-- Les membres d'un groupe passent par la clé primaire.
CREATE INDEX idx_contact_group_member_address
    ON contact_group_member (address_id, group_id) WHERE date_removed IS NULL;

-- ------------------------------------------------------------
-- Conversations (JWZ), une par compte : le même mail reçu sur deux comptes
-- fait deux conversations (modele de donnees.md, « thread »).
--
-- Version 9 : le dossier de la conversation est une colonne de thread, et
-- la vue matérialisée thread_folder (une ligne par conversation et par
-- dossier) disparaît. Une conversation n'occupe qu'un vrai dossier
-- (dossiers.md, « Dossier d'une conversation ») ; Sent, Drafts, Quarantine
-- et Deleted ne sont que des drapeaux techniques sur certains de ses mails.
-- Les listes lisent toutes cette seule table, par ses index.
--
-- folder : code d'une lettre du rôle (FolderRole::code) :
--   i Inbox, t Todo, w Waiting, a Archive, n Newsletter, p Paperwork,
--   x Junk, b Banned (vrais dossiers), s Sent, r Drafts, q Quarantine,
--   d Deleted (repli, quand aucun mail n'est dans un vrai dossier).
--   Règle de calcul, du premier cas qui s'applique au dernier :
--   1. le dossier du dernier mail rangé dans Inbox, Todo, Waiting, Archive,
--      Newsletter ou Paperwork ;
--   2. sinon celui du dernier mail rangé dans Junk ou Banned ;
--   3. sinon, s'il y a un mail dans l'un d'eux, Sent, puis Drafts, puis
--      Quarantine, puis Deleted (ordre fixe) ;
--   4. sinon NULL : la conversation n'a de mails que dans des dossiers sans
--      rôle (un dossier qui a perdu son rôle). Elle n'apparaît que dans
--      Everything.
-- has_sent, has_draft, has_quarantine, has_deleted : la conversation a au
--   moins un mail dans ce dossier technique. Les listes de ces dossiers
--   lisent le drapeau, pas folder : une conversation de l'Inbox à laquelle
--   l'utilisateur a répondu est aussi dans la liste Sent.
-- Agrégats « vivants » : date_last_received, message_count et unread_count
--   ne comptent pas les mails de Deleted, Junk et Banned ; une conversation
--   qui n'a que de tels mails les compte tous (sinon elle n'aurait ni date
--   ni nombre à afficher dans ces dossiers). date_last_modified compte tous
--   les mails : supprimer ou rejeter un mail est une modification.
-- Tous sont recalculés par le noyau (refresh_thread) dans la transaction
-- qui change les mails, et quand un dossier change de rôle.
-- ------------------------------------------------------------
CREATE TABLE thread (
    id             INTEGER PRIMARY KEY,
    account_id     INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    subject        TEXT NOT NULL DEFAULT '',
    date_last_received INTEGER NOT NULL DEFAULT 0,
    message_count  INTEGER NOT NULL DEFAULT 0,
    unread_count   INTEGER NOT NULL DEFAULT 0,
    -- Plus récente modification des mails de la conversation, tous dossiers
    -- confondus : tri par modification (version 2).
    date_last_modified INTEGER NOT NULL DEFAULT 0,
    -- Version 9, en fin de table (ALTER TABLE ADD COLUMN, sans réécriture).
    folder         TEXT CHECK (folder IN ('i','t','w','a','n','p','x','b','s','r','q','d')),
    has_sent       INTEGER NOT NULL DEFAULT 0 CHECK (has_sent IN (0,1)),
    has_draft      INTEGER NOT NULL DEFAULT 0 CHECK (has_draft IN (0,1)),
    has_quarantine INTEGER NOT NULL DEFAULT 0 CHECK (has_quarantine IN (0,1)),
    has_deleted    INTEGER NOT NULL DEFAULT 0 CHECK (has_deleted IN (0,1))
) STRICT;

-- Chaque liste est un parcours d'index, du plus récent au plus ancien, qui
-- s'arrête à la page : une requête, quel que soit le nombre de comptes ou
-- de dossiers. Pour chaque portée, une variante « tous les comptes » et une
-- variante « un compte », chacune triée par réception et par modification.
-- Les index sont croissants et parcourus à l'envers (voir idx_message_list).

-- Everything (ui.md) : toutes les conversations (version 2).
CREATE INDEX idx_thread_received ON thread (date_last_received, id);
CREATE INDEX idx_thread_account_received ON thread (account_id, date_last_received, id);
CREATE INDEX idx_thread_modified ON thread (date_last_modified, id);
CREATE INDEX idx_thread_account_modified ON thread (account_id, date_last_modified, id);

-- Un vrai dossier, de tous les comptes (vue « all ») ou d'un seul.
CREATE INDEX idx_thread_role_received ON thread (folder, date_last_received, id);
CREATE INDEX idx_thread_role_account_received ON thread (folder, account_id, date_last_received, id);
CREATE INDEX idx_thread_role_modified ON thread (folder, date_last_modified, id);
CREATE INDEX idx_thread_role_account_modified ON thread (folder, account_id, date_last_modified, id);

-- All Mail : tout sauf les conversations rangées dans Deleted, Junk ou
-- Banned. Index partiels : la requête doit reprendre la condition telle
-- quelle (queries.rs, ALL_MAIL), sinon SQLite ne s'en sert pas.
CREATE INDEX idx_thread_live_received ON thread (date_last_received, id)
    WHERE folder NOT IN ('d','x','b');
CREATE INDEX idx_thread_live_account_received ON thread (account_id, date_last_received, id)
    WHERE folder NOT IN ('d','x','b');
CREATE INDEX idx_thread_live_modified ON thread (date_last_modified, id)
    WHERE folder NOT IN ('d','x','b');
CREATE INDEX idx_thread_live_account_modified ON thread (account_id, date_last_modified, id)
    WHERE folder NOT IN ('d','x','b');

-- Version 11. Work (ui.md, « Dossiers virtuels ») : Inbox, Todo et Waiting
-- ensemble. Même principe que All Mail : la requête reprend la condition
-- telle quelle (queries.rs, WORK). Sans eux, folder IN ('i','t','w') lirait
-- trois intervalles d'idx_thread_role_received, puis les trierait.
CREATE INDEX idx_thread_work_received ON thread (date_last_received, id)
    WHERE folder IN ('i','t','w');
CREATE INDEX idx_thread_work_account_received ON thread (account_id, date_last_received, id)
    WHERE folder IN ('i','t','w');
CREATE INDEX idx_thread_work_modified ON thread (date_last_modified, id)
    WHERE folder IN ('i','t','w');
CREATE INDEX idx_thread_work_account_modified ON thread (account_id, date_last_modified, id)
    WHERE folder IN ('i','t','w');

-- Dossiers techniques : index partiels sur leur drapeau, qui ne contiennent
-- que les conversations concernées.
CREATE INDEX idx_thread_sent_received ON thread (date_last_received, id) WHERE has_sent = 1;
CREATE INDEX idx_thread_sent_account_received ON thread (account_id, date_last_received, id) WHERE has_sent = 1;
CREATE INDEX idx_thread_sent_modified ON thread (date_last_modified, id) WHERE has_sent = 1;
CREATE INDEX idx_thread_sent_account_modified ON thread (account_id, date_last_modified, id) WHERE has_sent = 1;
CREATE INDEX idx_thread_draft_received ON thread (date_last_received, id) WHERE has_draft = 1;
CREATE INDEX idx_thread_draft_account_received ON thread (account_id, date_last_received, id) WHERE has_draft = 1;
CREATE INDEX idx_thread_draft_modified ON thread (date_last_modified, id) WHERE has_draft = 1;
CREATE INDEX idx_thread_draft_account_modified ON thread (account_id, date_last_modified, id) WHERE has_draft = 1;
CREATE INDEX idx_thread_quarantine_received ON thread (date_last_received, id) WHERE has_quarantine = 1;
CREATE INDEX idx_thread_quarantine_account_received ON thread (account_id, date_last_received, id) WHERE has_quarantine = 1;
CREATE INDEX idx_thread_quarantine_modified ON thread (date_last_modified, id) WHERE has_quarantine = 1;
CREATE INDEX idx_thread_quarantine_account_modified ON thread (account_id, date_last_modified, id) WHERE has_quarantine = 1;
CREATE INDEX idx_thread_deleted_received ON thread (date_last_received, id) WHERE has_deleted = 1;
CREATE INDEX idx_thread_deleted_account_received ON thread (account_id, date_last_received, id) WHERE has_deleted = 1;
CREATE INDEX idx_thread_deleted_modified ON thread (date_last_modified, id) WHERE has_deleted = 1;
CREATE INDEX idx_thread_deleted_account_modified ON thread (account_id, date_last_modified, id) WHERE has_deleted = 1;

-- ------------------------------------------------------------
-- Messages bruts (stockage-brut.md) : fichiers immuables adressés par leur
-- contenu, dans <data_dir>/raw.noindex/. Une ligne par fichier, partagée par
-- tous les messages qui ont exactement les mêmes octets.
-- ------------------------------------------------------------
CREATE TABLE raw_blob (
    hash         BLOB PRIMARY KEY,          -- SHA-256 des octets bruts, 32 octets
    size         INTEGER NOT NULL,          -- taille d'origine (octets)
    stored_size  INTEGER NOT NULL,          -- taille sur disque (compressée zstd)
    stored_at    INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT, WITHOUT ROWID;

-- ------------------------------------------------------------
-- Messages : en-têtes seulement (table étroite = liste rapide).
-- Le corps vit dans message_body (nicemailer_body.db).
-- ------------------------------------------------------------
CREATE TABLE message (
    id               INTEGER PRIMARY KEY,
    account_id       INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    folder_id       INTEGER NOT NULL REFERENCES folder(id) ON DELETE CASCADE,
    -- Adresse du message sur le serveur, utilisée par la synchro et le spool,
    -- jamais exposée dans le contrat. L'identité stable est id.
    -- NULL : déplacement en cours, UID dans le dossier cible pas encore connu ;
    -- toujours NULL dans un compte d'archives, qui n'a pas de serveur.
    uid              INTEGER,
    thread_id       INTEGER REFERENCES thread(id) ON DELETE SET NULL,

    message_id_hdr   TEXT,                -- en-tête Message-ID, sans chevrons
    in_reply_to      TEXT,
    subject          TEXT NOT NULL DEFAULT '',
    -- Expéditeur. from_address_id sert aux recherches par expéditeur (rangement
    -- après une décision, filtre de confidentialité de la synchronisation) ;
    -- from_name et from_addr, tels que dans l'en-tête, sont dénormalisés pour
    -- afficher la liste sans jointure. NULL : en-tête From absent ou illisible.
    from_address_id  INTEGER REFERENCES address(id),
    from_name        TEXT,
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
    -- Message brut (stockage-brut.md) ; NULL tant que le message complet n'a
    -- pas été reçu. body_state = 1 suppose raw_hash renseigné.
    raw_hash         BLOB REFERENCES raw_blob(hash) ON DELETE RESTRICT,

    -- Traçabilité. date_modified ne suit que dossier, flags, tags et brouillon ;
    -- uid, modseq, body_state, snippet n'y touchent pas.
    -- date_last_viewed : dernière consultation dans un client humain (app, TUI,
    -- web) d'au moins 2 s ; ce n'est pas une modification, et elle reste locale.
    date_created     INTEGER NOT NULL DEFAULT (unixepoch()),
    date_modified    INTEGER NOT NULL DEFAULT (unixepoch()),
    created_by       TEXT NOT NULL CHECK (created_by  IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    modified_by      TEXT NOT NULL CHECK (modified_by IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    date_last_viewed INTEGER,

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

-- Vues "derniers mails traités" et "derniers mails consultés", tous dossiers
-- confondus (voir tracabilite.md).
CREATE INDEX idx_message_modified ON message (date_modified);
-- Version 9. Critère « Date mail… » de toutes les conversations
-- (Everything) : les conversations qui ont un mail reçu dans l'intervalle.
-- Dans une liste de dossiers, idx_message_list (folder_id, date_received)
-- sert le même critère.
CREATE INDEX idx_message_received ON message (date_received);
CREATE INDEX idx_message_viewed
    ON message (date_last_viewed) WHERE date_last_viewed IS NOT NULL;

-- Messages d'une conversation, et ceux d'une conversation dans un dossier
-- (aperçu d'une ligne de liste), dans l'ordre chronologique. Sans folder_id
-- après thread_id, SQLite lisait tout le dossier pour chaque ligne de la
-- liste (version 2).
CREATE INDEX idx_message_thread
    ON message (thread_id, folder_id, date_received, id);

-- Filtres « avec drapeau », « répondu » et « pièce jointe » (ui.md) : les
-- conversations concernées sont lues dans ces index partiels, qui ne
-- contiennent que les mails marqués, au lieu d'interroger chaque
-- conversation de la liste (version 2). La pièce jointe porte le dossier :
-- elle ne compte que dans les dossiers listés, d'où folder_id en tête.
CREATE INDEX idx_message_thread_flagged
    ON message (thread_id) WHERE is_flagged = 1;
CREATE INDEX idx_message_thread_answered
    ON message (thread_id) WHERE is_answered = 1;
CREATE INDEX idx_message_thread_attachments
    ON message (folder_id, thread_id) WHERE has_attachments = 1;

-- Mails d'un expéditeur, par dossier : rangement après une décision d'adresse
-- ou de domaine, « adresse vue dans cette boîte » (synchronisation-configuration.md).
CREATE INDEX idx_message_from
    ON message (from_address_id, folder_id) WHERE from_address_id IS NOT NULL;

-- Retrouver un message par Message-ID (threading, déduplication).
CREATE INDEX idx_message_msgid
    ON message (account_id, message_id_hdr);

-- Messages qui partagent un brut (ramasse-miettes, déduplication de l'import).
CREATE INDEX idx_message_raw_hash
    ON message (raw_hash, account_id) WHERE raw_hash IS NOT NULL;

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
-- Préfixe $Nm réservé au noyau : $NmWasTodo, $NmWasWaiting marquent une
-- conversation revenue dans l'Inbox (dossiers.md). Comparaison sans casse.
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
-- Un tag ne peut pas utiliser un mot-clé du préfixe réservé $Nm.
-- Un mot-clé IMAP est un atome ASCII (ni espace, ni parenthèses, ni accents) :
-- `name` est le nom affiché, `keyword` sa forme encodée envoyée au serveur.
-- ------------------------------------------------------------
CREATE TABLE tag (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,                  -- nom affiché, libre
    keyword     TEXT NOT NULL COLLATE NOCASE,   -- mot-clé IMAP (insensible à la casse)
    color       TEXT,
    date_deleted INTEGER,               -- pierre tombale : tag supprimé, masqué partout
    date_created INTEGER NOT NULL DEFAULT (unixepoch()),
    date_modified INTEGER NOT NULL DEFAULT (unixepoch()),
    created_by  TEXT NOT NULL CHECK (created_by  IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    modified_by TEXT NOT NULL CHECK (modified_by IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    UNIQUE (account_id, name),
    UNIQUE (account_id, keyword)
) STRICT;

-- ------------------------------------------------------------
-- Corps et recherche plein texte : dans le second fichier,
-- nicemailer_body.db (schema_body.sql, version 10). Voir modele de donnees.md,
-- « Deux fichiers ».
-- ------------------------------------------------------------

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
-- Désabonnement d'une liste (ui.md, « Désabonnement »), extrait à la
-- réception. Table séparée : peu de messages en ont un, et la table message,
-- lue par chaque liste, reste étroite.
--   source 'header' : en-tête List-Unsubscribe (RFC 2369), fiable ;
--   source 'body'   : lien du corps dont le texte annonce un désabonnement,
--                     ouvert dans le navigateur seulement.
--   one_click       : List-Unsubscribe-Post (RFC 8058) avec une URL https :
--                     le noyau envoie lui-même la requête.
-- ------------------------------------------------------------
CREATE TABLE message_unsubscribe (
    message_id  INTEGER PRIMARY KEY REFERENCES message(id) ON DELETE CASCADE,
    url         TEXT,                     -- https ou http ; NULL si mailto seul
    mailto      TEXT,                     -- adresse mailto: complète, en-tête seulement
    one_click   INTEGER NOT NULL DEFAULT 0 CHECK (one_click IN (0,1)),
    source      TEXT NOT NULL CHECK (source IN ('header','body')),
    CHECK (url IS NOT NULL OR mailto IS NOT NULL)
) STRICT;

-- ------------------------------------------------------------
-- Brouillons en cours de rédaction (voir brouillons.md).
-- La ligne draft est la source pendant la rédaction ; la copie dans le dossier
-- Drafts du serveur n'est écrite qu'à la fermeture de la fenêtre.
-- Modifié localement : revision <> server_revision (ou server_revision NULL).
-- message_id : la ligne message (une seule par brouillon) qui représente la
-- copie sur le serveur ; NULL tant qu'aucune copie n'existe.
-- Pas de journal des modifications dans audit_log, seulement l'envoi.
-- ------------------------------------------------------------
CREATE TABLE draft (
    id                  INTEGER PRIMARY KEY,
    account_id          INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    from_addr           TEXT NOT NULL,
    from_name           TEXT,
    recipients          TEXT NOT NULL DEFAULT '[]',  -- JSON : [{kind, addr, name}], kind to|cc|bcc
    subject             TEXT NOT NULL DEFAULT '',
    body                TEXT NOT NULL DEFAULT '',
    body_format         TEXT NOT NULL DEFAULT 'text' CHECK (body_format IN ('text','html')),
    reply_to_message_id INTEGER REFERENCES message(id) ON DELETE SET NULL,
    -- Rangement de la conversation après l'envoi (dossiers.md), choisi par
    -- ⇧⌘↩ ; 'stay' : laissée en place (option d'essai, reponse.md) ; NULL = présélection.
    after_send          TEXT CHECK (after_send IN ('archive','waiting','todo','stay')),
    message_id_hdr      TEXT NOT NULL,    -- Message-ID stable, généré à la création
    message_id          INTEGER REFERENCES message(id) ON DELETE SET NULL,
    revision            INTEGER NOT NULL DEFAULT 1,
    server_revision     INTEGER,          -- révision de la dernière copie sur le serveur
    state               TEXT NOT NULL DEFAULT 'editing' CHECK (state IN ('editing','sending')),
    date_created        INTEGER NOT NULL DEFAULT (unixepoch()),
    date_modified       INTEGER NOT NULL DEFAULT (unixepoch()),
    created_by          TEXT NOT NULL CHECK (created_by  IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    modified_by         TEXT NOT NULL CHECK (modified_by IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    -- Le mail transféré (f) ; NULL pour une réponse ou un nouveau mail.
    -- Un transfert n'a pas de reply_to_message_id : il ne range rien.
    forward_of_message_id INTEGER REFERENCES message(id) ON DELETE SET NULL,
    -- Le compte qui envoie (son SMTP, son dossier Sent), quand ce n'est pas
    -- account_id : réponse à un mail d'un compte qui ne peut pas envoyer
    -- (archive, IMAP sans [smtp]), ou expéditeur changé (reponse.md, 4 ter).
    -- NULL = account_id. Le brouillon reste dans la conversation d'account_id.
    send_account_id     INTEGER REFERENCES account(id) ON DELETE SET NULL,
    UNIQUE (message_id_hdr)
) STRICT;

CREATE INDEX idx_draft_message ON draft (message_id) WHERE message_id IS NOT NULL;

-- Fichiers joints aux brouillons, copiés dans outgoing.noindex/ à l'ajout,
-- adressés par le SHA-256 de leur contenu (voir brouillons.md, section 7).
CREATE TABLE outgoing_blob (
    hash        BLOB PRIMARY KEY,         -- SHA-256, 32 octets
    size        INTEGER NOT NULL,
    stored_at   INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT, WITHOUT ROWID;

CREATE TABLE draft_attachment (
    id          INTEGER PRIMARY KEY,
    draft_id    INTEGER NOT NULL REFERENCES draft(id) ON DELETE CASCADE,
    hash        BLOB NOT NULL REFERENCES outgoing_blob(hash) ON DELETE RESTRICT,
    filename    TEXT NOT NULL,            -- nom affiché et envoyé
    mime_type   TEXT NOT NULL,
    position    INTEGER NOT NULL,         -- ordre d'affichage et d'envoi
    content_id  TEXT,                     -- image insérée dans le corps HTML (cid:)
    is_inline   INTEGER NOT NULL DEFAULT 0 CHECK (is_inline IN (0,1))
) STRICT;

CREATE INDEX idx_draft_attachment_draft ON draft_attachment (draft_id, position);
CREATE INDEX idx_draft_attachment_hash  ON draft_attachment (hash);

-- ------------------------------------------------------------
-- Spool : file d'actions locales à rejouer vers le serveur (hors-ligne).
-- L'action est déjà appliquée localement à l'insertion (affichage immédiat).
-- Erreur temporaire : reste 'queued', date_run_after repoussé.
-- Erreur définitive : 'reverted', l'état local est restauré depuis undo.
-- 'cancelled' : annulée sans exécution (fusion, ou annulation en série).
-- Voir modele de donnees.md, section 6.
-- ------------------------------------------------------------
CREATE TABLE pending_action (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN
                ('set_flags','move','copy','delete','append_draft','send','push_config','initial_archive')),
    -- push_config : écriture de la configuration synchronisée, sans message ;
    -- contenu calculé à l'exécution (voir synchronisation-configuration.md).
    -- initial_archive : archivage groupé côté serveur à la création du compte,
    -- sans message ; critères dans payload (voir import-initial.md).
    -- append_draft : copie d'un brouillon dans Drafts, payload = {draft_id, revision}
    -- (voir brouillons.md, section 5).
    message_id  INTEGER REFERENCES message(id) ON DELETE SET NULL,
    payload     TEXT NOT NULL,            -- JSON : move = {from_folder_id, from_uid, to_folder_id}
    undo        TEXT,                     -- JSON : état local à restaurer en cas d'échec définitif
    state       TEXT NOT NULL DEFAULT 'queued'
                CHECK (state IN ('queued','running','done','reverted','cancelled')),
    attempts    INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT,
    date_created   INTEGER NOT NULL DEFAULT (unixepoch()),
    date_modified  INTEGER NOT NULL DEFAULT (unixepoch()),  -- dernier changement d'état
    created_by     TEXT NOT NULL CHECK (created_by IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    date_run_after INTEGER NOT NULL DEFAULT 0  -- backoff
) STRICT;

CREATE INDEX idx_pending_action_queue
    ON pending_action (account_id, state, date_run_after, id);

-- Actions en attente sur un message : fusion, exécution en série par message,
-- et consultation par la synchro (UID à ignorer, lignes à rattacher).
CREATE INDEX idx_pending_action_message
    ON pending_action (message_id, id) WHERE state IN ('queued','running');

-- ------------------------------------------------------------
-- Journal de traçabilité : historique des décisions et des actions, en ajout
-- seul, jamais modifié. Pas de clé étrangère : l'historique survit à la
-- suppression de l'entité. Les changements de source 'imap' n'y figurent pas.
-- Purgé après audit.retention_days. Voir tracabilite.md.
-- ------------------------------------------------------------
CREATE TABLE audit_log (
    id           INTEGER PRIMARY KEY,
    date_created INTEGER NOT NULL DEFAULT (unixepoch()),
    created_by   TEXT NOT NULL CHECK (created_by IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
    entity       TEXT NOT NULL CHECK (entity IN
                 ('account','folder','address','domain','contact_group','tag','message','config')),
    entity_id    INTEGER NOT NULL,        -- 'config' : 0 ; la clé est dans detail
    action       TEXT NOT NULL,           -- ex. screening, auth_policy, role, move, set_flags
    detail       TEXT                     -- JSON : {"before": …, "after": …, …}
) STRICT;

CREATE INDEX idx_audit_log_entity ON audit_log (entity, entity_id, date_created);
CREATE INDEX idx_audit_log_date   ON audit_log (date_created);

-- ------------------------------------------------------------
-- Réglages protégés : hors de config.toml et d'accounts.toml, modifiables seulement par l'app
-- (client vérifié par sa signature). Voir configuration.md, section 7.
-- Clés : 'mcp.allow_modify', 'web.enabled'. Valeur JSON.
-- ------------------------------------------------------------
CREATE TABLE protected_setting (
    key           TEXT PRIMARY KEY,
    value         TEXT NOT NULL,
    date_modified INTEGER NOT NULL DEFAULT (unixepoch()),
    modified_by   TEXT NOT NULL CHECK (modified_by = 'app')
) STRICT, WITHOUT ROWID;

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
-- Recherche plein texte (message_fts, dans nicemailer_body.db), triée par pertinence :
--   SELECT m.id, m.subject, snippet(message_fts, 3, '<b>', '</b>', '…', 12)
--     FROM message_fts
--     JOIN message m ON m.id = message_fts.rowid
--    WHERE message_fts MATCH :q          -- ex. 'factur*' ou 'sender:dupont AND devis'
--    ORDER BY rank
--    LIMIT 50;
--
-- Liste de conversations d'un vrai dossier, tous comptes confondus :
--   SELECT t.id, t.subject, t.date_last_received, t.message_count, t.unread_count
--     FROM thread t
--    WHERE t.folder = 'i'
--      AND (t.date_last_received, t.id) < (:cursor_date, :cursor_id)
--    ORDER BY t.date_last_received DESC, t.id DESC
--    LIMIT 50;
