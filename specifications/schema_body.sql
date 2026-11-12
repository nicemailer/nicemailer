-- ============================================================
-- NiceMailer : second fichier de base, nicemailer_body.db (version 10 du
-- schéma principal). Corps des mails et recherche plein texte.
-- Analyse et justification : modele de donnees.md, « Deux fichiers ».
--
-- Tout ce fichier se reconstruit à partir des messages bruts
-- (raw.noindex/, stockage-brut.md) et de nicemailer.db : supprimé, il est
-- recréé vide et rempli en tâche de fond par la réinterprétation (reindex),
-- comme après un changement d'analyseur. La sauvegarde porte donc sur
-- nicemailer.db et raw.noindex/ seulement.
--
-- Le noyau l'attache à chaque connexion sous le nom « body »
-- (ATTACH DATABASE … AS body) : les requêtes nomment message_body et
-- message_fts sans préfixe, SQLite les trouve dans ce fichier. Une
-- transaction qui écrit dans les deux fichiers n'est atomique que fichier par
-- fichier (mode WAL) : le noyau rapproche les deux à l'ouverture
-- (lignes orphelines supprimées, lignes manquantes refaites).
-- ============================================================

PRAGMA journal_mode = WAL;
-- Version de ce fichier, indépendante de celle de nicemailer.db.
PRAGMA user_version = 1;

-- ------------------------------------------------------------
-- Corps : interprétation du message brut (seul le brut sur disque est immuable).
-- Écrit à la réception du message complet, puis réécrit par reindex quand
-- l'analyseur ou l'assainissement évoluent. Ce n'est pas un cache : il n'est
-- jamais évincé, seule la suppression du message l'efface.
-- message_id = message.id de nicemailer.db. Pas de clé étrangère : SQLite
-- n'en permet pas entre deux fichiers. La suppression d'un message efface
-- son corps et sa ligne de recherche par un trigger temporaire de chaque
-- connexion qui écrit (lib.rs, TEMP_TRIGGERS), et le rapprochement à
-- l'ouverture rattrape ce qu'un arrêt brutal aurait laissé.
-- ------------------------------------------------------------
CREATE TABLE message_body (
    message_id      INTEGER PRIMARY KEY,
    text_plain      TEXT,
    html_sanitized  TEXT,                 -- déjà passé par ammonia ; aucune source qu'une vue web
                                          -- chargerait seule : images distantes en nm-remote:,
                                          -- pixels de suivi sans source (remote_images.rs)
    -- 1 : le HTML contient des images distantes chargeables à la demande
    -- (hors pixels de suivi).
    has_remote_content INTEGER NOT NULL DEFAULT 0 CHECK (has_remote_content IN (0,1)),
    parser_version  INTEGER NOT NULL,     -- version de l'analyseur et de l'assainissement ;
                                          -- reindex cible les lignes de version inférieure
    date_fetched    INTEGER NOT NULL DEFAULT (unixepoch()),
    -- Pixels de suivi distants retirés du HTML.
    tracking_pixels INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX idx_message_body_parser ON message_body (parser_version);

-- ------------------------------------------------------------
-- Recherche plein texte. rowid = message.id.
-- remove_diacritics 2 : "eleve" trouve "élève".
-- ------------------------------------------------------------
CREATE VIRTUAL TABLE message_fts USING fts5 (
    subject, sender, recipients, body,
    tokenize = 'unicode61 remove_diacritics 2',
    prefix   = '2 3'
);

-- Insertion : pas de trigger. Le noyau insère la ligne de message_fts lui-même,
-- en une seule instruction par message, avec l'objet, l'expéditeur, les
-- destinataires et le corps s'il est déjà connu (sinon vide, complété à
-- l'arrivée du corps). Raison : dans une transaction, chaque instruction qui
-- déclenche un trigger sur une table FTS5 ouvre un point de sauvegarde, et
-- FTS5 vide alors son tampon et fusionne ses segments ; avec les triggers
-- d'insertion, l'import d'un mbox de 46 000 mails passait plus de la moitié
-- de son temps d'écriture dans ces fusions. Une insertion directe est deux
-- fois plus rapide (mesure du 3 novembre 2026).
--
-- Les triggers qui suivent la table message (changement d'objet ou
-- d'expéditeur, suppression) relient les deux fichiers : ils ne peuvent pas
-- vivre dans l'un d'eux, et sont créés TEMP à l'ouverture de chaque
-- connexion qui écrit (lib.rs, TEMP_TRIGGERS).

-- Quand le corps est réinterprété (reindex) : mettre l'index à jour.
-- reindex procède par UPDATE, jamais par INSERT OR REPLACE (qui supprimerait
-- puis réinsérerait la ligne sans passer par ce trigger).
CREATE TRIGGER message_body_fts_au AFTER UPDATE OF text_plain ON message_body BEGIN
    UPDATE message_fts SET body = coalesce(new.text_plain, '') WHERE rowid = new.message_id;
END;
