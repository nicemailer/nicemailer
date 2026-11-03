# NiceMailer : stockage des messages bruts

Statut : validé. Sections 3 à 5 implémentées pour l'import mbox (`import-initial.md`, section 9) : écriture, dédoublonnage, table `raw_blob`, `message.raw_hash`. Les sections 6 à 9 (lecture des pièces jointes, ramasse-miettes, `reindex`, `export`) restent à implémenter. Complète `schema.sql` et `modele de donnees.md`.

## 1. Objet et principe

Ce document spécifie comment le noyau Rust conserve le message original (RFC 5322 complet, tel que reçu du serveur) et comment il l'utilise.

Le message brut est une **archive immuable adressée par son contenu** :

- il est écrit **une seule fois**, quand sa réception est complète ;
- son nom est le hash de son contenu : il ne dépend ni du dossier, ni des flags, ni du compte ;
- **aucune opération ultérieure** (changement de flag, déplacement, copie, étiquette) ne le touche : tout cela ne concerne que SQLite ;
- il n'est lu que dans trois cas : téléchargement d'une pièce jointe, ré-analyse (`reindex`), export.

Tout ce qui sert à l'affichage (en-têtes, destinataires, corps texte et HTML assaini, liste des pièces jointes) est extrait à la réception et stocké dans SQLite. L'ouverture d'un message et le défilement de la liste ne lisent jamais le brut.

## 2. Objectifs et non-objectifs

**Objectifs**

- Immuabilité et intégrité vérifiable.
- Dédoublonnage : un même message présent dans plusieurs dossiers ou étiquettes n'occupe qu'un fichier.
- Taille réduite sur disque (compression).
- Écriture atomique et reprise sûre après plantage.
- Noyau indépendant de l'UI et réutilisable (CLI, TUI, MCP).

**Non-objectifs**

- Lisibilité du dossier à la main ou lecture par d'autres clients mail. L'interopérabilité passe par la commande `export` (section 9).
- Accès aléatoire à l'intérieur d'un message.
- Chiffrement applicatif (FileVault protège le disque ; voir questions ouvertes).

## 3. Disposition sur disque

```
<data_dir>/
  nicemailer.db
  raw.noindex/              ← ".noindex" : exclu de Spotlight
    .tmp/                   ← fichiers en cours d'écriture
    .corrupt/               ← blobs corrompus (section 6)
    ab/                     ← 2 premiers caractères hexadécimaux du hash
      abcdef…0123.eml.zst   ← hash SHA-256 en hexadécimal minuscule (64 caractères)
  attachments.noindex/      ← pièces jointes extraites (section 6), purgeables
    ab/                     ← 2 premiers caractères hexadécimaux du hash du brut
      abcdef…0123/          ← hash du brut dont la pièce jointe est extraite
        1.2/                ← part_id
          facture-mars.pdf  ← nom assaini, extension conservée
  outgoing.noindex/         ← pièces jointes des brouillons, copiées à l'ajout (brouillons.md)
    ab/
      abcdef…0123           ← hash SHA-256 du contenu, sans compression
  lock                      ← verrou d'instance unique
  core.sock                 ← socket du noyau, créé par lui (architecture.md, section 6)
```

`<data_dir>` est `~/Library/Application Support/NiceMailer/`.

- Le dossier `.tmp` est sur le **même volume** que le reste, pour que le `rename` soit atomique.
- Fichiers en `0400`, dossiers en `0700`.
- Un seul niveau de répartition (256 sous-dossiers) suffit jusqu'à environ 100 000 messages. Au-delà, passer à deux niveaux (`ab/cd/`). Le changement de disposition est une migration.
- Le hash est calculé sur les **octets bruts reçus** (avant compression), sans aucune normalisation (fins de ligne CRLF conservées).
- Le noyau prend un verrou exclusif sur `lock` au démarrage. Une seconde instance sur la même base refuse de démarrer.

## 4. Modèle de données

À intégrer au schéma (version 1 si elle n'est pas encore diffusée, sinon migration vers `user_version = 2`).

```sql
CREATE TABLE raw_blob (
    hash         BLOB PRIMARY KEY,          -- SHA-256, 32 octets
    size         INTEGER NOT NULL,          -- taille d'origine (octets)
    stored_size  INTEGER NOT NULL,          -- taille sur disque (compressée zstd)
    stored_at    INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT, WITHOUT ROWID;

-- Dans message :
--   raw_hash BLOB REFERENCES raw_blob(hash) ON DELETE RESTRICT
-- NULL tant que le message complet n'a pas été téléchargé.

CREATE INDEX idx_message_raw_hash ON message (raw_hash) WHERE raw_hash IS NOT NULL;
```

Changements par rapport au schéma actuel :

- `message_body.raw_path` est **supprimé**. Le brut n'est plus rattaché au corps.
- `message_body` est une **interprétation** du brut : écrit à l'ingestion, réécrit par `reindex` (section 8), jamais évincé. Seul le brut est immuable.
- `message.body_state` : 0 = en-têtes seuls (pas de brut), 1 = message complet. Le passage à 1 suppose que `raw_hash` est renseigné.
- Le nombre de références à un blob n'est pas stocké (risque de dérive). Il est calculé à la demande : `SELECT 1 FROM message WHERE raw_hash = ?`.

Des messages de dossiers différents peuvent partager le même `raw_hash`. Si deux copies d'un même `Message-ID` diffèrent d'un octet, ce sont deux blobs distincts : c'est voulu.

## 5. Écriture (ingestion)

Déclencheur : le message complet est entièrement reçu. Un message dont la réception est partielle ou interrompue **n'est jamais** enregistré.

1. Recevoir le littéral IMAP en flux. Écrire en continu dans `.tmp/<aléatoire>` en calculant le SHA-256 sur les octets bruts et en les compressant au vol. Rien n'est conservé entièrement en mémoire.
2. À la fin, vérifier que le nombre d'octets reçus égale la taille annoncée par le serveur. Sinon, supprimer le temporaire et signaler l'erreur.
3. `fsync` du fichier temporaire. Sur macOS, `fsync` confie les données au disque sans vider son cache ; seul `F_FULLFSYNC` le vide, pour environ 10 ms que le disque sérialise. Le noyau fait donc un `fsync` simple par fichier, puis un seul `F_FULLFSYNC` par lot, avant de valider la transaction qui référence les fichiers du lot : la garantie est la même (aucune ligne `raw_blob` sans fichier sur support stable), pour un vidage par lot au lieu de deux par message. Avec deux `F_FULLFSYNC` par message, l'import d'un mbox plafonnait à 22 Mo/s.
4. Si `<hash>.eml.zst` existe déjà : supprimer le temporaire (dédoublonnage). Sinon : `rename` vers le chemin final, puis `fsync` du dossier parent.
5. Analyser le message (en-têtes, corps, pièces jointes) et ouvrir **une transaction SQLite** qui :
   - fait `INSERT OR IGNORE` dans `raw_blob` ;
   - met à jour ou insère `message` (avec `raw_hash`), `message_address`, `message_reference`, `attachment`, `message_body` ;
   - met à jour les compteurs dénormalisés concernés.
6. Valider la transaction.

Cas particuliers :

| Situation | Comportement |
|---|---|
| Plantage entre 4 et 6 | Fichier sans ligne en base : orphelin inoffensif, supprimé par le balayage (section 7). |
| Ligne `raw_blob` présente mais fichier absent | Réécrire le fichier à partir du message reçu (auto-réparation). |
| Fichier présent mais ligne absente | Insérer la ligne (le fichier est adopté). |
| Échec de l'analyse MIME | Le brut est quand même conservé et lié ; le message est marqué en erreur d'analyse pour pouvoir être retraité par `reindex`. |

**Niveau de compression** : paramètre (`raw.zstd_level`), fixé à 9 par mesure sur 300 Mo d'un export réel, sur un cœur : niveau 3, 640 Mo/s pour 60 % de la taille d'origine ; niveau 9, 120 Mo/s pour 55 % ; niveau 15, 18 Mo/s pour un gain négligeable. Sur l'export mbox complet (9,7 Go), les blobs occupent 5,7 Go. La compression s'exécute hors du chemin d'affichage et la lecture est rare : on peut viser un niveau élevé, tant que le débit d'ingestion reste supérieur à celui du réseau.

## 6. Lecture

Le brut n'est lu que par :

1. **Téléchargement d'une pièce jointe** :
   1. retrouver `attachment.part_id` et `message.raw_hash` ;
   2. ouvrir le fichier, décompresser en flux, vérifier le SHA-256 pendant la lecture ;
   3. repérer la partie MIME (la correspondance entre `part_id` IMAP et l'analyseur doit être couverte par des tests) ;
   4. décoder le transfert (base64, quoted-printable) et écrire dans un fichier temporaire ;
   5. poser l'attribut `com.apple.quarantine`, renommer vers `attachments.noindex/<hash>/<part_id>/<nom assaini>` et renseigner `attachment.disk_path`.

   Les fichiers extraits sont rangés par hash du brut et non par message : deux copies d'un même message (deux dossiers) partagent la même extraction. Ils sont dérivés du brut et peuvent être purgés à tout moment ; `attachment.disk_path` est alors remis à NULL et la prochaine demande réextrait le fichier. La politique de purge automatique reste à définir. Le transfert aux clients par chemin est décrit dans `architecture.md`, section 5.
2. **`reindex`** (section 8).
3. **`export`** (section 9).

Un échec de vérification du hash renvoie l'erreur `RawCorrupt`. Le fichier est déplacé dans `raw.noindex/.corrupt/`. Si le message existe encore sur le serveur (via `folder_id` et `uid`), il est retéléchargé et réécrit. Sinon, l'erreur est remontée à l'utilisateur.

Si le fichier est absent alors que la base le référence, le comportement est le même que pour la corruption (retéléchargement si possible).

## 7. Suppression et ramasse-miettes

« Immuable » signifie que le contenu d'un blob ne change jamais. Un blob peut en revanche disparaître quand plus aucun message ne le référence.

**GC des blobs sans référence**, déclenché périodiquement et après de grosses suppressions :

1. Dans une transaction, supprimer de `raw_blob` les lignes où `NOT EXISTS (SELECT 1 FROM message WHERE raw_hash = raw_blob.hash)`. Conserver la liste des hashes supprimés.
2. Après validation, supprimer les fichiers correspondants, ainsi que les dossiers `attachments.noindex/<hash>/` des pièces jointes extraites de ces blobs.

Les pièces jointes des brouillons (`outgoing.noindex/`, table `outgoing_blob`) suivent la même règle : une ligne `outgoing_blob` sans `draft_attachment` qui la référence est supprimée, puis son fichier, sous le même verrou.

**Balayage des orphelins**, au démarrage et périodiquement : parcourir `raw.noindex/`, supprimer
- tout fichier de `.tmp/` de plus de 24 heures ;
- tout fichier final sans ligne `raw_blob` et dont la date de modification dépasse un délai de grâce (par exemple 24 heures).

**Concurrence entre ingestion et GC** : dans le noyau, un verrou partagé/exclusif sérialise les deux. L'ingestion le prend en mode partagé de l'étape 4 à la validation de la transaction ; le GC le prend en mode exclusif. Cela évite le cas où l'ingestion constate que le fichier existe, puis où le GC le supprime avant que la ligne soit validée. L'auto-réparation de la section 6 couvre tout reliquat.

Une suppression de fichier ne garantit pas l'effacement sécurisé des données sur le disque.

## 8. Reconstruction (`reindex`)

Commande qui relit les blobs et régénère, pour tout ou partie des messages : `message_body`, `snippet`, `attachment`, `message_address`, `message_reference`, entrées FTS. `message_body` est créé s'il est absent (échec d'analyse initial) et réécrit par `UPDATE` s'il existe.

- Sert après une amélioration de l'analyseur (parsing des en-têtes, destinataires, pièces jointes) ou de l'assainissement HTML (correction de faille, détection des pixels de suivi).
- Cible par défaut les messages dont `message_body.parser_version` est inférieur à la version courante ; une montée de version de l'analyseur déclenche donc la réinterprétation en tâche de fond, hors du chemin d'affichage.
- Une correction de sécurité de l'assainissement doit être appliquée en priorité : tant qu'un corps n'a pas été réinterprété, il est réassaini à la lecture.
- Ne touche jamais au brut.
- Ne recalcule pas le verdict d'authentification (`auth_result`, `auth_suspect`) : il a servi à une décision de screening déjà prise (voir `authentification-expediteur.md`).
- Ne reconstitue pas dossiers, UID ni flags : ceux-ci viennent d'une resynchronisation IMAP. En cas de perte de la base, le brut seul ne suffit donc pas à tout restaurer, mais il conserve le contenu des messages.

## 9. Export et interopérabilité

Commande `export` du noyau (CLI) qui produit, à partir de la base et des blobs :

- m2dir, Maildir ou mbox ;
- sélection par compte, dossier ou requête de recherche ;
- fichiers `.eml` décompressés individuels.

C'est le seul endroit où l'on respecte un format de stockage externe. Le stockage interne reste indépendant de ces formats.

## 10. Paramètres

Ces paramètres sont des clés de `config.toml` ; le catalogue complet est dans `configuration.md`, section 11.

| Paramètre | Rôle | Valeur initiale |
|---|---|---|
| `raw.dir` | Racine du stockage brut | `<data_dir>/raw.noindex` |
| `raw.zstd_level` | Niveau de compression | 9 (constante du code pour l'instant) |
| `raw.fanout` | Niveaux de sous-dossiers | 1 |
| `raw.gc_grace_secs` | Délai de grâce des orphelins | 86400 |
| `raw.verify_on_read` | Vérifier le hash à la lecture | vrai |

## 11. Tests exigés

- Écriture concurrente du même message depuis deux dossiers : un seul fichier, deux lignes `message`.
- Plantage simulé à chaque étape de la section 5 : état cohérent après redémarrage.
- Corruption d'un octet : détection, mise à l'écart dans `.corrupt/`, retéléchargement.
- GC concurrent avec une ingestion du même hash.
- Correspondance `part_id` IMAP / analyseur MIME sur un corpus varié (multipart imbriqué, messages transférés, pièces jointes sans nom).
- Messages volumineux (> 100 Mo) : mémoire constante.
- Disque plein pendant l'écriture : aucune ligne en base, aucun fichier partiel.
- `reindex` complet : résultat identique à une ingestion initiale avec l'analyseur courant, `parser_version` à jour, index plein texte cohérent avec `message_body`.

## 12. Questions ouvertes

1. **Niveau zstd et dictionnaire** : mesurer le gain d'un dictionnaire entraîné sur des en-têtes de mails.
2. **Chiffrement au repos** : s'appuyer sur FileVault, ou prévoir un chiffrement par blob plus tard (ce qui compliquerait le dédoublonnage).
3. **Seuil de passage aux segments** : si la volumétrie atteint plusieurs millions de messages, évaluer un stockage par segments avec compactage.
4. **Décalage des pièces jointes** : stocker offset et longueur de chaque partie dans `attachment` pour éviter de réanalyser le message à chaque téléchargement.
5. **Algorithme de hash** : SHA-256 retenu (standard, accéléré sur Apple Silicon) ; BLAKE3 est une option sans enjeu fonctionnel.
