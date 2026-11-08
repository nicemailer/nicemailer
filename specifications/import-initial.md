# NiceMailer : import initial d'un compte

Statut : validé. Complète `dossiers.md` (création du compte, screening).

## 1. Objet

À la création d'un compte, l'Inbox du serveur contient souvent des milliers de mails. Les envoyer tous en Quarantine noierait l'utilisateur dès sa première expérience du produit ; les laisser tous dans l'Inbox lui ôterait tout intérêt.

Principes :

- **Rien n'est perdu** : les mails anciens sont archivés, jamais supprimés.
- **Les mails déjà présents ne sont pas triés.** Ils restent dans l'Inbox ou sont archivés, mais aucun ne part en Quarantine.
- **Les prochains mails sont triés normalement.** Un expéditeur sans décision va en Quarantine, y compris s'il figure dans l'historique. L'utilisateur fait ainsi son ménage progressivement, une décision à la fois. Chaque décision range aussi les anciens mails de l'expéditeur (`dossiers.md`, « Décision sur un expéditeur »).
- **Exception pour les correspondants.** Les personnes à qui l'utilisateur a déjà écrit arrivent directement dans l'Inbox (option cochée par défaut, section 4).

## 2. Déroulement

1. **Identifiants** : connexion de test (étape existante, `dossiers.md`).
2. **Dossiers** : correspondance entre les dossiers du serveur et ceux du workflow (étape existante).
3. **Comptage** : `STATUS INBOX (MESSAGES UNSEEN UIDNEXT)`. Le `UIDNEXT` obtenu fixe la frontière entre mails existants et nouveaux mails (section 5).
4. **Synchronisation des en-têtes de Sent**, pour compter les correspondants (section 4).
5. **Écran d'import** (section 3), affiché si l'Inbox dépasse un seuil ou s'il y a des correspondants à proposer.
6. **Archivage côté serveur** (section 6).
7. **Synchronisation initiale** des dossiers, qui commence par l'Inbox, puis le reste. Les mails déjà sur le serveur ne passent pas par le screening.

À partir de là, chaque nouveau mail est trié.

## 3. L'écran d'import

> Votre boîte de réception contient 18 052 mails, dont 3 921 non lus.
>
> ☑ Archiver les mails **lus** antérieurs au [aujourd'hui]
> ☑ Archiver les mails **non lus** antérieurs au [il y a 30 jours]
> Les mails marqués d'un drapeau restent dans la boîte de réception.
>
> → 17 960 mails seront archivés, 92 resteront dans la boîte de réception.
>
> ☑ Les personnes à qui vous avez déjà écrit (284) arriveront directement dans la boîte de réception.
>
> Les autres expéditeurs passeront par la Quarantine à leur prochain mail.

- **Seuils** : les options d'archivage ne sont proposées que si l'Inbox contient plus de `import.threshold_total` mails (100) ou plus de `import.threshold_unseen` non-lus (50). En dessous, l'Inbox reste telle quelle.
- **Dates** : par défaut, aujourd'hui pour les lus (tous) et 30 jours (`import.unseen_default_age_days`) pour les non-lus. L'utilisateur peut les changer.
- **Décompte exact** : chaque changement de date ou de case recalcule le nombre de mails concernés par `UID SEARCH`, avec `RETURN (COUNT)` si le serveur gère ESEARCH. Sinon, le noyau compte les UID renvoyés.
- **Mails marqués d'un drapeau** (`\Flagged`) : jamais archivés. L'utilisateur les a signalés comme importants.

## 4. Les correspondants

Les destinataires (To, Cc, Bcc) des mails du dossier Sent sont des correspondants réels : on n'écrit pas à un spammeur.

- Le noyau synchronise d'abord les en-têtes de Sent, puis compte les adresses distinctes parmi les destinataires, en excluant les adresses du compte lui-même (adresse principale et identités, `configuration.md`).
- Si la case est cochée, chacune de ces adresses **sans décision** reçoit la décision `inbox` (`auth_policy` reste `auto`). Une adresse qui a déjà une décision, par exemple venue d'un autre compte ou de la synchronisation entre Mac, la garde.
- Ces décisions sont prises par le noyau (`created_by = 'core'`) et journalisées une par une dans `audit_log`, avec la raison `sent_correspondent`.
- Après l'import, la même règle s'applique au fil de l'eau, sans option : écrire à quelqu'un vaut décision Inbox (`dossiers.md`).

## 5. Frontière entre l'existant et le nouveau

La frontière est un UID, pas une date : `folder.uid_screening_start` de l'Inbox reçoit le `UIDNEXT` lu au comptage (étape 3).

- UID inférieur : mail importé, jamais trié.
- UID supérieur ou égal : nouveau mail, trié normalement. Un mail arrivé pendant que l'utilisateur remplit l'écran d'import est donc trié.
- Si l'`UIDVALIDITY` de l'Inbox change avant la fin de la synchronisation initiale (cas rare), tout le contenu présent est considéré comme importé : `uid_screening_start` reprend le nouveau `UIDNEXT`.

Tant que `uid_screening_start` est NULL (compte en cours de création), le screening de ce compte est suspendu. Provisoirement, jusqu'à l'écran d'import de la phase 7 (`plan-imap.md`), la synchronisation le fixe elle-même au `UIDNEXT` de l'Inbox lu à sa première synchronisation.

## 6. Archivage côté serveur

Il ne faut pas télécharger 18 000 en-têtes dans l'Inbox pour lancer ensuite 18 000 déplacements. L'archivage est exécuté **sur le serveur, avant la synchronisation de l'Inbox** : chaque mail est ingéré une seule fois, directement dans son dossier final.

- Une seule action de spool `initial_archive`, sans message (`message_id` NULL), dont le `payload` porte les critères : dossiers source et cible, dates, frontière d'UID.
- Exécution : `UID SEARCH` avec `UID 1:<frontière - 1>`, `NOT FLAGGED`, `SEEN BEFORE <date des lus>` puis `UNSEEN BEFORE <date des non-lus>` ; puis `UID MOVE` par lots de `import.batch_size` UID (1 000), pour éviter les commandes trop longues et les délais dépassés. `BEFORE` porte sur la date interne, la même que `date_received`.
- **Reprise** : l'action est rejouable. Après une interruption, elle refait la recherche, qui ne renvoie plus les mails déjà déplacés.
- **Erreur temporaire** : nouvelle tentative, selon les règles du spool. La synchronisation initiale de l'Inbox attend la fin de l'action ; l'app affiche l'avancement.
- **Erreur définitive** (dossier Archive introuvable, par exemple) : l'app l'affiche et propose de reprendre ou de passer l'archivage. Rien n'a été modifié localement : il n'y a rien à annuler.
- **Journal** : une seule ligne `initial_archive` sur l'entité `account` dans `audit_log`, avec les critères et le nombre de mails déplacés, et non une ligne par mail.

## 7. Adresses importées

`address.imported_from` indique qu'une adresse a été découverte par un import, et lequel :

| Valeur | Origine |
| --- | --- |
| NULL | Découverte par la réception normale |
| `imap_initial` | Import initial d'un compte IMAP (ce document) |
| `mbox` | Import d'une archive mbox (section 9) |

- La valeur est renseignée quand l'import **crée** l'adresse (expéditeur ou destinataire d'un mail importé). Une adresse déjà connue garde sa valeur.
- Elle n'est jamais effacée, même après une décision : c'est de l'historique.
- **Usage dans la Quarantine** : un expéditeur importé est présenté avec son historique (« présent dans votre historique : 342 mails depuis 2015 »), un expéditeur inconnu comme tel. La première situation est presque toujours une vraie relation, approuvée en un clic ; la seconde peut être du spam.

Le routage n'a pas besoin de cet attribut : une adresse sans décision va déjà en Quarantine.

## 8. Traçabilité

| Élément | Valeur |
| --- | --- |
| Messages ingérés par la synchronisation initiale | `created_by = 'imap'` : ils existaient sur le serveur |
| Décisions des correspondants | `created_by = 'core'`, une ligne `screening` par adresse dans `audit_log` |
| Archivage initial | Une ligne `initial_archive` sur le compte, avec la source du client qui a validé l'écran |
| Messages importés d'un mbox | `created_by` = client qui a lancé l'import (`cli`) |
| Import mbox | Une ligne `create` (compte d'archives créé) puis une ligne `import_mbox` par import, sur le compte (section 9) |

## 9. Import d'une archive mbox

Statut : implémenté (CLI seulement). Un fichier mbox (export de Apple Mail, de Gmail, de Thunderbird…) est importé dans un **compte d'archives**.

### Compte d'archives

- Un compte d'archives (`account.kind = 'archive'`) n'a **jamais** de serveur IMAP ni SMTP. Il n'est pas dans `accounts.toml` : son nom (`account.config_key`) et l'adresse de la boîte archivée (`account.address`) sont donnés à l'import.
- Il reçoit tous les dossiers du workflow. Les mails importés peuvent être rangés comme les autres (Todo, Paperwork, Deleted…) ; les déplacements ne modifient que la base locale et ne créent aucune action de spool.
- Un nom déjà pris par un compte IMAP est refusé. Un compte d'archives existant est réutilisé si l'adresse est la même, refusé sinon : l'adresse décide de ce qui va dans Sent et ne peut pas changer d'un import à l'autre.

### Classement

| Mail | Dossier | Lu |
| --- | --- | --- |
| Expéditeur (`From`) égal à l'adresse du compte, sans casse | Sent | oui |
| Tous les autres | Archive | oui |

Un mbox ne porte ni dossier ni flag fiable : tout est archivé, comme les mails anciens de l'import initial (section 1). Aucun mail ne passe par le screening ni n'arrive en Quarantine.

Autres attributs :

- `date_received` : date de la ligne de séparation `From ` du mbox (date de réception par le serveur exportateur, équivalent de l'INTERNALDATE), à défaut l'en-tête `Date`, à défaut 0. La date de la ligne `From ` est écartée au profit de l'en-tête `Date` quand elle ne peut pas être une date de livraison : postérieure au début de l'import, ou postérieure de plus de 7 jours à l'en-tête `Date` (un serveur SMTP abandonne au bout de 5 jours environ). Certains outils d'export y écrivent la date de l'export (le module `mailbox` de Python le fait si on ne lui donne pas de date) : sans cette règle, tous les mails paraîtraient reçus le jour de l'export.
- `is_answered` : posé sur un mail reçu auquel un mail de Sent du même compte répond (`In-Reply-To`).
- `uid` : NULL (pas de serveur).
- `auth_result` : `unknown`, `auth_source` NULL. Un export ne garantit pas que les en-têtes `Authentication-Results` viennent d'un serveur de confiance ; dans l'export HEY, ils viennent de plusieurs serveurs (`haystack-mail-inbound-postfix-1` à `-8`), et la détection de `authserv_id` (`authentification-expediteur.md`) échouerait.
- `created_by` : le client qui a lancé l'import (`cli`).

### Conversations

Un mail rejoint la conversation de tout mail du même compte qu'il référence (`References`, `In-Reply-To`), qui le référence, ou qui a le même `Message-ID`. Les mails d'un export ne sont pas dans l'ordre des conversations : une réponse peut précéder le mail auquel elle répond, et un mail peut relier deux conversations existantes, qui sont alors fusionnées. Les mails sans référence ne sont pas regroupés par objet : cela fusionnerait à tort des mails sans rapport, comme des factures mensuelles de même objet. L'objet de la conversation est celui de son mail le plus ancien.

### Stockage

Chaque mail est conservé tel quel (`stockage-brut.md`), après retrait de l'échappement mbox (`>From `). Les fins de ligne du fichier sont conservées sans normalisation. Le corps texte, le HTML assaini et les pièces jointes sont extraits comme pour un mail reçu.

### Déduplication et reprise

- Un mail dont les octets bruts (même SHA-256) sont déjà dans le compte est ignoré. Relancer l'import du même fichier n'ajoute donc rien, et un import interrompu se reprend en le relançant.
- Deux copies d'un même `Message-ID` qui diffèrent d'un octet sont deux mails : c'est la règle du stockage brut. Elles rejoignent la même conversation.
- L'écriture se fait par lots de `import.mbox_batch_size` mails, chacun dans une transaction : une interruption perd au plus le lot en cours, sans état intermédiaire visible.

### Adresses et correspondants

- Les adresses créées par l'import reçoivent `imported_from = 'mbox'` (section 7) ; le nom d'affichage est celui du premier mail du fichier qui cite l'adresse.
- Les destinataires (To, Cc, Bcc) des mails de Sent sont des correspondants : comme à la section 4, ceux qui n'ont pas de décision reçoivent `inbox` (`created_by = 'core'`, une ligne `screening` dans `audit_log` avec la raison `sent_correspondent`). L'adresse du compte est exclue. Une adresse dont la décision a été retirée (pierre tombale) n'est pas modifiée : l'import ne revient pas sur un choix de l'utilisateur. L'option est active par défaut ; `--no-correspondents` la désactive.

### Rangement par les décisions futures

Les adresses de l'archive seront revues : par le prochain mail de l'expéditeur sur un compte IMAP, ou dans la Quarantine. La règle « Décision sur un expéditeur » (`dossiers.md`) s'applique aussi aux comptes d'archives : classer en Paperwork le prochain mail de `invoice@amazon.fr` range aussi dans Paperwork ses mails importés qui sont dans le dossier Archive du compte d'archives. Les mails de Sent ne bougent pas.

**Exception à la condition d'authentification.** Une décision favorable ne déplace d'ordinaire que les mails dont le verdict est accepté (`dossiers.md`). Les mails d'un compte d'archives ont tous le verdict `unknown`, qui n'est plus accepté dès que l'adresse a été vue authentifiée (`auth_seen = 1`). Ils sont donc rangés **sans condition d'authentification** : ce sont des mails anciens, déjà lus, que l'utilisateur a jugés en les gardant, et le pire cas (un ancien mail usurpé rangé dans Paperwork plutôt qu'Archive) est sans gravité. *À implémenter avec les décisions de screening, qui ne sont pas encore codées.*

### Exécution

- L'import est exécuté par le noyau (`import_mbox`), qui seul écrit dans la base ; le CLI ne fait que le demander et suivre son avancement : `nicemailer import-mbox FICHIER --account gmail --address bob@gmail`.
- Un seul import à la fois (erreur `BUSY` sinon). L'avancement est publié par les événements `ImportProgress`, un par lot, et la fin par `ImportFinished` (`evenements.md`). `import_status` renvoie l'état de l'import en cours ou du dernier.
- Interrompre le CLI n'interrompt pas l'import. Arrêter le noyau l'interrompt ; il se reprend en le relançant.
- Le fichier est lu en flux : la mémoire utilisée dépend du plus gros mail, pas de la taille du fichier.
- Mesure (MacBook, 10 cœurs, fichier de 9,7 Go et 46 539 mails) : 36 s, dont environ 6 s de lecture et d'analyse, 9 s de compression et le reste d'écriture SQLite. Le relancer sur le même fichier prend 6 s.

### Journal

- Une ligne `create` sur l'entité `account` à la création du compte d'archives.
- Une ligne `import_mbox` sur l'entité `account` à chaque import, avec le fichier et les nombres de mails importés, ignorés, non analysés, envoyés, archivés, et de correspondants. Pas de ligne par mail.

### Hors périmètre pour l'instant

- Import depuis l'app (écran d'import) : le CLI seulement.
- Dépôt des mails sur un serveur (`APPEND`) : un compte d'archives n'a pas de serveur.
- Lecture des flags d'un mbox (`Status:`, `X-Status:`, `X-Gmail-Labels`) : tout est marqué lu et classé selon l'expéditeur.
- Plusieurs adresses (identités) pour un compte d'archives.

## 10. Paramètres

Ces paramètres sont des clés de `config.toml` ; le catalogue complet est dans `configuration.md`, section 11.

| Paramètre | Rôle | Valeur initiale |
| --- | --- | --- |
| `import.threshold_total` | Nombre de mails de l'Inbox au-delà duquel l'archivage est proposé | 100 |
| `import.threshold_unseen` | Nombre de non-lus au-delà duquel l'archivage est proposé | 50 |
| `import.unseen_default_age_days` | Âge par défaut au-delà duquel les non-lus sont archivés | 30 |
| `import.batch_size` | Nombre d'UID par `UID MOVE` | 1 000 |
| `import.mbox_batch_size` | Nombre de mails écrits par transaction lors d'un import mbox (section 9) ; constante du code pour l'instant | 500 |

## 11. Tests exigés

- Inbox sous les seuils, sans correspondants : pas d'écran, aucun mail déplacé, aucun mail en Quarantine.
- Archivage : seuls les mails correspondant aux critères sont déplacés ; les mails marqués d'un drapeau restent.
- Décompte affiché égal au nombre de mails réellement déplacés.
- Mail arrivé pendant l'écran d'import : trié normalement.
- Interruption au milieu de l'archivage : reprise sans doublon ni perte.
- Changement d'`UIDVALIDITY` pendant l'import : aucun mail existant trié.
- Correspondants : décision `inbox` pour les adresses sans décision ; une décision existante est conservée ; les adresses du compte sont exclues.
- `imported_from` renseigné pour les adresses créées par l'import, inchangé pour les adresses déjà connues.
- Après l'import, premier mail d'un expéditeur importé sans décision : Quarantine, présenté avec son historique.

Import mbox (section 9) :

- Lecture du mbox : séparation sur les lignes `From ` précédées d'une ligne vide et portant une date, déséchappement mboxrd, fins de ligne CRLF conservées, fichier qui n'est pas un mbox refusé sans rien importer.
- Classement Sent / Archive selon l'expéditeur, sans casse ; tous les mails lus.
- Réponse importée avant le mail auquel elle répond : même conversation ; mail qui relie deux conversations : fusion.
- Réimport du même fichier : aucun mail ajouté, tous comptés comme déjà présents.
- Brut conservé octet pour octet (après déséchappement), relu et vérifié par son hash.
- Correspondants : `inbox` pour les destinataires des mails envoyés sans décision ; décision existante conservée ; adresse du compte exclue ; option désactivable.
- `imported_from = 'mbox'` pour les adresses créées, inchangé pour les adresses connues.
- Nom de compte déjà pris par un compte IMAP, ou compte d'archives existant avec une autre adresse : refusé.
- Import par le socket : chemin relatif refusé, événements `ImportProgress` et `ImportFinished`, état disponible par `import_status`.
