Il y a trois versions ou thèmes pour l'application Swift NiceMailer : classic, design et geek.

Le thème classique s'approche des canons historiques du design d'une application macOS et est destiné à faciliter la transition depuis une application telle que Apple Mail. 

Le thème design cherche à offrir une interface sobre et élégante privilégiant la lisibilité.  Elle est conçue pour les utilisateurs privilégiant l'esthétique. 

Le thème geek est une interface minimaliste conçue comme un IDE de programmation.


Quelques exemples de différences :

- la couleur de fond et les arrondis délimitant les zones de l’interface peuvent être marqués dans la version classique, sobres dans la version design et inexistante dans la version geek.
- les ascenseurs sont standard dans la version classique, plus sobres dans la version design et inexistante dans la version geek
- La version classique à une barre d’outils en haut de l’interface avec des icônes de boutons alors que dans la version geek, c'est une barre de statut plus compacte, qui peut être en bas, en haut ou masquée. Dans cette version geek, chaque élément présent dans la barre de statut peut être ouvert avec un raccourci clavier d'une seule touche et affiche alors un menu contextuel dans le même style que Helix-editor.
- Dans la version classique, l'animation pour révéler la barre de gauche avec les différents dossiers est clairement visible. Elle est très rapide et subtile sur la version design et absolument inexistante sur la version geek où aucune milliseconde n'est perdue. 

Pendant le développement, il y a un menu thème qui permet de basculer d'un thème à l'autre. Ce sera plus tard une option de configuration. 

État (3 novembre 2026) : le thème geek est implémenté ; classic correspond à la première interface ; design n'existe pas encore et affiche classic.

### Thème geek : choix retenus

- Police monospace pour l'interface (SF Mono) ; corps des mails en **iA Writer Mono V**, livrée avec l'app (licence SIL OFL).
- Palette inspirée de Helix, en variante claire et sombre selon l'apparence de macOS.
- HTML de l'expéditeur : sa mise en page et son CSS sont gardés, assainis par le noyau (aucune adresse distante chargeable, rien qui dépende du survol ou de la taille de l'écran), chaque message dans un cadre isolé : son style ne touche ni les autres messages ni l'interface. Un mail qui prévoit son mode sombre s'affiche avec ses couleurs, sombres dans le thème sombre. Les autres, faits pour une page blanche, sont adaptés au thème sombre par Dark Reader (licence MIT) : fond du thème, texte clair, teintes des boutons et des titres gardées ; il tourne isolé de la page, sans aucun accès au réseau, et s'efface dans le thème clair. Les polices web ne sont pas chargées. Les images intégrées au mail (`cid:`) s'affichent en html comme en images : elles ne demandent rien au réseau.
- Corps des mails en texte par défaut. Un sélecteur `text | html | images` en haut du mail, et `vh`, basculent en HTML assaini ; `images` (ou `vi`) charge aussi les images distantes (voir « Images distantes »). Le choix est mémorisé **par adresse d'expéditeur** (`address.body_format`, `address.remote_images`, `schema.sql`).
- Compte unifié **all** : avec plusieurs comptes, l'app s'ouvre sur « all », qui regroupe tous les comptes. Chaque dossier (Inbox, Archive…) y réunit les dossiers de même rôle de tous les comptes, avec leurs compteurs additionnés. La colonne des dossiers n'a qu'un bloc, intitulé « all » ou le nom du compte choisi ; le sélecteur de dossiers et le menu `;` ne montrent que les dossiers de cette vue, sans notion de compte.
- Barre de statut : le mode (l'icône de l'application en mode normal ; sinon le mode écrit : `EDT` pendant l'écriture d'une réponse, le préfixe en attente comme dans Helix, `PCK` dans un sélecteur, `ASK` pour une question), puis le compte (icône @, « all » pour la vue unifiée) et le dossier (icône de dossier), chacun suivi du rappel de sa touche. Chaque élément s'ouvre d'une touche ou d'un clic : le mode avec `?` (liste des touches), le compte avec `@` (sélecteur de comptes, « all » en tête : on arrive dans le dossier de même rôle, sinon l'Inbox), le dossier avec `;` (menu des dossiers de la vue, d'« Everything » et des écrans, qui se déroule à gauche depuis le bloc dossier de la barre de statut : en dessous si la barre est en haut, au-dessus si elle est en bas ; ses lignes se cliquent aussi). Les menus `m` (déplacer), `s` (enregistrer) et `v` (affichage) ont aussi leur bloc dans la barre de statut, une icône suivie de la touche. Un large espace sépare trois groupes : ce que la liste affiche (`@`, `;`, `f` et les pastilles), la préférence d'affichage (`v`), les actions sur le mail courant (`m`, `s`). Leur menu se déroule depuis ce bloc de la même façon. À l'extrémité droite de la barre, sur tous les écrans, un bloc `?` (point d'interrogation cerclé) ouvre la liste des touches comme la touche `?` ; un second clic la ferme. Il rappelle l'aide à qui ne connaît pas encore la touche. Dans un menu, les flèches surlignent une ligne (bas commence à la première, haut à la dernière, la liste tourne) et entrée va à la ligne surlignée ; une touche agit toujours directement. Barre masquée : les menus s'affichent en bas à droite. `h` ne fait plus rien : il faisait double emploi avec `;`. Pendant qu'un tag, un groupe ou un contact est listé, la barre l'affiche à la place. Les messages (résultat d'une action, erreur) s'affichent à droite et disparaissent à la touche suivante. Un déplacement nomme le mail et le dossier : « Devis toiture » déplacé dans Archive (sujet coupé à 50 caractères) ; pour plusieurs mails, leur nombre : 3 conversations déplacées dans Archive.
- Les sélecteurs (`@`, `l`, `t`, `g`, `f`) sont des listes filtrables à la manière de Helix : on tape pour filtrer (chaque mot tapé doit apparaître), `ctrl-n` / `ctrl-p` ou les flèches pour se déplacer, entrée pour choisir, échap pour fermer. À l'ouverture, la première ligne est sélectionnée ; la liste tourne : flèche haut (ou `ctrl-p`) depuis la première ligne sélectionne la dernière, flèche bas (ou `ctrl-n`) depuis la dernière revient à la première. Pour `t` et `g`, taper un nom absent propose de le créer. Le texte tapé s'édite comme dans tout champ macOS : flèches gauche et droite pour déplacer le curseur (⌥ par mot, ⌘ jusqu'au bout), effacement des deux côtés du curseur, et ⌘V colle le texte copié dans une autre app, sur une seule ligne, sans les espaces autour. Un clic en dehors d'un sélecteur, d'un menu ou de l'aide le ferme, comme échap, sans rien faire d'autre.

L'interface utilisateur de NiceMailer est, quel que soit le thème choisi, minimaliste et sobre. 

La fenêtre principale permet de lister les mails en deux colonnes : 
- A gauche, sur un petit tiers de l'écran, mais resizable, la liste des mails. 
- A droite, sur le reste de l'écran, l'affichage du mail sélectionné à gauche. 

Lors de l'ouverture d'une fenêtre, le premier mail est systématiquement sélectionné dans la liste, de sorte que la partie de droite affiche quelque chose. 

### Liste

- Chaque ligne : le point des non lus, la date, l'expéditeur, un trombone s'il y a des pièces jointes, le sujet et ses tags, (dans Everything) le dossier, le nombre de messages de la conversation, le drapeau.
- Le trombone est seul, sans chiffre, et placé avant le sujet : à côté du nombre de messages en fin de ligne, « 📎 3 » se lirait comme trois pièces jointes.
- Le drapeau est une icône orange en fin de ligne, couleur propre, distincte du point jaune des non lus.
- ⌘A marque tous les mails de la liste telle qu'affichée (les lignes chargées jusqu'ici), pour une action groupée ; la barre de statut dit combien ; Échap les démarque. Seulement sur l'écran des mails, sans panneau ouvert.

#### Actions groupées

Une action sur plusieurs mails (les mails marqués) part en **une seule** demande au noyau, quelle que soit leur nombre : lu et non lu (`u`, `U`), drapeau (`*`), tag (`t`), déplacement (`d`, `m`, Suppr, les cibles directes de Quarantine). Le noyau les traite dans une seule transaction et n'émet qu'un lot d'événements. La liste change d'un coup : lu, non lu et drapeau s'affichent tout de suite sur toutes les lignes, avant la réponse du noyau, et les mails déplacés quittent la liste ensemble ; en cas d'erreur, la liste est relue et montre l'état du noyau. Avant (novembre 2026), l'app envoyait une demande par mail, et les lignes changeaient l'une après l'autre. Un déplacement groupé s'annule d'un seul `z`, en une demande. Un mail dont l'expéditeur est suspect d'usurpation n'est pas déplacé avec les autres : il revient dans la liste et l'app pose la question pour lui seul ; un mail pour lequel la cible n'est pas permise reste aussi où il est.

### Affichage d'une conversation

L'affichage est simplifié au maximum : chaque message montre ce qu'il apporte. Le noyau découpe chaque message et l'app n'affiche que le résultat (détail : `docs/thread-display.md`).

- Masqués par défaut : la citation finale (historique cité, avec sa ligne « … a écrit : » ou le bloc d'en-têtes d'Outlook), le texte qui recopie un message précédent, les signatures (après `-- `, ou lignes finales répétées par un expéditeur), les mentions (« Envoyé de mon iPhone », avis de confidentialité). Chaque partie masquée est remplacée par une ligne « ··· » cliquable.
- Les citations intercalées entre les réponses restent visibles, atténuées, avec une barre par niveau. Un message transféré n'est jamais masqué.
- Comme dans Gmail, le dernier message et les non-lus sont dépliés ; les autres tiennent sur une ligne (expéditeur, date, début du texte). Un message qui ne fait que citer l'indique.
- En-têtes allégés : expéditeur et date ; les destinataires seulement s'ils changent d'un message à l'autre.
- `T` (comme dans mutt) affiche ou masque tout ce que la vue simplifiée cache, pour la conversation ouverte ; `S` va au texte qui suit la citation suivante ; `ctrl-k` remonte au message précédent de la conversation et `ctrl-j` redescend au suivant, un seul déplié à la fois : le message courant se replie, l'autre se déplie et la lecture va à son en-tête (au départ, le dernier message déplié, ou le dernier en-tête cliqué ; arrêt aux deux bouts, que la barre de statut signale ; avec `T`, rien ne se replie, on passe d'un en-tête à l'autre). Un clic sur un en-tête plie ou déplie le message, un clic sur « ··· » affiche cette partie.

### Pièces jointes

- Au pied de chaque mail déplié, une dalle par pièce jointe : l'icône de son type (SF Symbol, aux couleurs du thème : image, PDF, texte, tableur, présentation, archive, audio, vidéo, code, agenda, contact, mail, fichier), le nom coupé au milieu pour garder l'extension, la taille. 3 à 5 dalles par ligne selon la largeur du panneau. Un mail plié montre un trombone et le nombre de pièces jointes.
- Un clic enregistre le fichier dans Téléchargements, sans jamais écraser un fichier existant (« devis (2).pdf ») ; la barre de statut le confirme. ⌘-clic l'enregistre de même, puis ouvre la copie enregistrée avec l'app par défaut, au premier plan.
- Les images affichées par le HTML du mail (`cid:`) ne sont pas des dalles. Une image marquée « intégrée » qu'aucun HTML n'affiche (photo d'Apple Mail dans un mail texte) en est une.
- Au clavier, `l` liste les liens du mail puis une ligne par fichier joint, avec son icône ; entrée enregistre le fichier, ⌘-entrée l'enregistre puis ouvre aussitôt la copie enregistrée, au premier plan.
- `l` cherche aussi les **expéditeurs du fil** dans le carnet d'adresses du Mac (les auteurs des messages reçus, pas les personnes en copie) : pour chaque fiche trouvée, une ligne par numéro de téléphone (« Alice Martin — mobile », entrée appelle) puis « Fiche de Alice Martin », qui ouvre Contacts. Un numéro déjà présent dans le mail n'apparaît qu'une fois, sous le nom. L'accès est demandé par macOS au premier `l` ; refusé, la barre de statut le dit une fois. Rien du carnet n'est copié dans la base.
- En HTML, les mêmes dalles suivent le corps du mail.

### Expéditeur

- Une seule ligne par message : « nom · date · adresse ». Message déplié : l'adresse complète (« ▾ Alice Martin · 30 oct. 14:00 · alice@atelier-dupuis.fr ») ; les destinataires, quand ils valent d'être montrés (À, Cc), sur la ligne suivante. Message replié : le domaine seul, puis l'aperçu (« ▸ Alice Martin · 30 oct. 11:00 · @atelier-dupuis.fr  Peux-tu… ») ; sans nom, l'adresse tient lieu de nom et n'est pas répétée. En texte comme en HTML.
- La flèche (▸ replié, ▾ déplié) précède le nom dans une conversation seulement : un clic sur la ligne replie ou déplie le message. Un mail seul n'a ni flèche ni repli : il est toujours déplié.
- Alerte : l'adresse passe dans la couleur d'alerte quand le mail a échoué à l'authentification, ou quand le nom affiché montre une autre adresse ou un autre domaine que le vrai (« service@paypal.com » <x7@promo-mail.ru>, « PayPal.com »). Un domaine du nom dont le vrai domaine se termine par lui ne compte pas (« Le Monde.fr » depuis news.lemonde.fr).
- Un clic sur l'adresse la copie (la barre de statut le dit) ; ⌘-clic filtre la liste sur cet expéditeur (pastille « de : »).
- Les destinataires gardent leur nom seul.

### Images distantes

Charger une image distante apprend à l'expéditeur que le mail a été ouvert, quand, et depuis quelle adresse IP. Les pixels de suivi n'ont pas d'autre usage. Règles :

- **Pixels de suivi retirés par le noyau** à la réception : une image distante de 2 pixels ou moins (`width`, `height` ou style), ou masquée par son style (`display: none`, `visibility: hidden`, `opacity: 0`), perd sa source et n'est jamais demandée, quel que soit l'affichage. Leur nombre s'affiche sur la ligne de titre : un œil barré suivi du nombre (l'infobulle dit « pixels bloqués : N »). La ligne de titre (sujet, désabonnement, pixels, text | html | images) passe sur deux lignes, le sujet seul puis les contrôles à droite, dès que le sujet entier ne tient plus à côté des contrôles. La détection se fait avant toute requête : une fois la requête partie, l'ouverture est signalée.
- **Autres images distantes** : leur adresse est conservée sous une forme que la vue web ne charge pas seule (`nm-remote:`). Le mot `images` du sélecteur n'apparaît que si la conversation en contient.
- **Mode `images`** (clic ou `vi`) : l'app télécharge chaque image une fois, en HTTPS uniquement (`http:` est converti, jamais envoyé en clair), sans cookies, sans `Referer`, avec un User-Agent neutre, jamais vers la machine ni le réseau local, 10 Mo au plus et seulement du contenu `image/…`. Les images sont gardées en cache : rouvrir un mail ne refait aucune requête. Seuls les messages **dépliés** chargent leurs images.
- **Mémorisé par expéditeur, appliqué aux seuls mails authentifiés** (`pass`, `authentification-expediteur.md`) : sans authentification, n'importe qui peut écrire au nom d'un expéditeur de confiance. Pour un autre mail (import compris), la conversation s'ouvre en HTML ; choisir `images` charge les images **pour cette conversation seulement**.
- **Jamais dans Junk, Quarantine et Banned**, même sur demande.
- Une barrière de plus dans la vue web : une liste de blocage WebKit refuse toute ressource `http(s)`, au cas où une source distante passerait l'assainissement.

Limites assumées : l'adresse IP et l'heure de la première ouverture restent visibles pour chaque image chargée ; un mail dont chaque image porte un jeton propre au destinataire signale l'ouverture dès qu'on charge ses images ; un pixel masqué par un élément parent, ou sans aucune taille déclarée, n'est pas reconnu. Le préchargement de toutes les images à la réception (à la manière d'Apple Mail) est écarté : il confirmerait l'adresse à tous les expéditeurs, spammeurs compris.

### Désabonnement

À gauche du sélecteur `text | html | images` de la ligne de titre, un bouton de désabonnement apparaît quand le dernier mail reçu de la conversation donne un moyen de se désabonner. Il est masqué dans Junk, Quarantine et Banned : se désabonner d'un spam confirme au spammeur que l'adresse est lue (le noyau refuse aussi).

- **Identification, par le noyau à la réception** : d'abord l'en-tête `List-Unsubscribe` (RFC 2369), fiable ; à défaut, un lien du corps dont le texte annonce un désabonnement (plusieurs langues). Le bouton distingue les deux : lien de l'en-tête, « propre », ou lien du corps, qui ouvre une page de l'expéditeur dans le navigateur.
- **Action** : avec `List-Unsubscribe-Post: List-Unsubscribe=One-Click` (RFC 8058), le noyau envoie lui-même la requête et la barre de statut confirme. Sinon la page s'ouvre dans le navigateur ; une adresse `mailto:` seule s'ouvre dans Mail en attendant l'envoi SMTP. Un échec du désabonnement en un clic ouvre la page.
- **Ensuite** : l'expéditeur est noté « désabonné le … » (affiché à la place du bouton, et journalisé), sans changer sa décision ni ses mails.
- **Clavier** : ⌘U, comme dans Superhuman ; le modificateur évite de déclencher par erreur une action irréversible (`u` seul bascule lu / non lu).

### Lecture

- Un mail choisi par l'utilisateur (`j` `k`, un clic, le mail suivant après un déplacement) est marqué lu après une seconde d'affichage. Parcourir vite la liste ne marque rien. En une colonne, la liste seule ne montre pas le contenu : rester sur une ligne ne marque rien. Ouvrir le mail (Entrée, double-clic) le marque lu aussitôt ; dans le mail ouvert, j / k suivent la règle de la seconde.
- Un mail sélectionné par l'app (le premier à l'ouverture d'une liste, ou celui qui remplace un mail sorti de la liste) n'est pas marqué lu tout seul. Il l'est quand l'utilisateur le quitte pour un autre mail de la liste après au moins une seconde. Le déplacer ou changer de dossier ne le marque pas.
- Dans une liste filtrée (non lus, avec drapeau…), le mail sélectionné qui ne correspond plus (lu après une seconde, `u`, `U`, `*`) reste sous le curseur, affiché avec son nouvel état, jusqu'à ce qu'on le quitte ; il sort alors de la liste. Sans cela, une liste de mails non lus se vidait toute seule, un mail par seconde.
- Consulter un mail de Quarantine ne le marque **jamais** lu, quelle que soit la règle ci-dessus et où qu'il soit listé (Everything compris) : on juge un expéditeur inconnu, on ne lit pas son mail, et le point non lu continue de dire quels mails attendent une décision. Règle de l'app : le noyau accepte toujours `set_thread_seen`, le CLI peut donc marquer lu sur demande explicite.

### Quarantine

En Quarantine, la seule chose à faire d'un mail est de le déplacer vers une cible (`dossiers.md`, « Cibles proposées »). Les cibles deviennent donc des touches directes, sans le préfixe `m` : on économise une touche sur le geste le plus répété du dossier.

- **Barre de statut** : les blocs `m` et `s` disparaissent ; à l'extrémité droite de la barre, juste avant `?`, la flèche de déplacement puis chaque cible suivie de sa touche : `Inbox i  Newsletter n  Paperwork p  Junk x  Banned b  Deleted d`. Alignées à droite, elles restent au même endroit quels que soient le compte, le dossier et les pastilles à gauche ; un message (résultat d'un déplacement) s'affiche avant elles et c'est lui qui est coupé si la place manque. Un clic sur une cible déplace comme sa touche.
- **Une touche déplace** : `i` fait ce que fait `mi` ailleurs, pour les mails marqués ou le mail sous le curseur ; `z` annule. `d` déplace dans **Deleted** (Archive n'est pas une cible depuis la Quarantine), alors qu'il archive partout ailleurs. Choix assumé : la règle « la lettre de la cible » prime sur le sens habituel de `d` ; Deleted se rattrape par `z`.
- **`m`** n'ouvre pas de menu : la barre de statut dit quelles touches taper.
- **Refusées** : `u` `U` (lu / non lu), `t` (tag), `g` (groupe), `r` (répondre : écrire à l'expéditeur l'accepterait, « Écrire à quelqu'un vaut décision Inbox »), `s` (enregistrer : son bloc quitte la barre ; un mail en Quarantine est à trier, pas à conserver). La barre de statut dit qu'en Quarantine un mail se déplace ou reçoit un drapeau, rien d'autre.
- **Restent** : `*` (drapeau), la navigation et les marques, `v`, `l`, `z`, Suppr.

Cela vaut quand un dossier Quarantine est listé (d'un compte, ou de « all »). Dans Everything ou une liste de tag, groupe ou contact, les lignes viennent de plusieurs dossiers : les blocs `m` et `s` et les touches habituelles restent.

### Sélecteur d'attributs

À droite du compte et du dossier, dans la barre de statut, une loupe (touche `f`, ou un clic ; `f` et `F` échangés en novembre 2026 : la recherche par attributs, d'usage constant, prend la minuscule, le transfert `F` la majuscule) ouvre le sélecteur d'attributs, qui affine la liste courante (dossier d'un compte, ou de « all », ou mails d'un tag, d'un groupe ou d'un contact, dont la barre de statut garde alors « tag : … » suivi de la loupe et des pastilles) ; les attributs se combinent par « et ».

- non lus / lus, drapeau / sans drapeau, pièce jointe / sans pièce jointe, répondus / sans ma réponse (répondu par l'utilisateur), conversations / mails seuls ;
- De…, À… (À et Cc ; une partie d'un nom ou d'une adresse, contacts suggérés pendant la frappe ; plusieurs valeurs combinées par « ou » ; le nom est celui du contact, `address.name`, et non le nom d'affichage de chaque mail : un mail envoyé par une adresse partagée, comme `notifications@github.com` au nom de Thomas, n'est pas « de : thomas »), Texte… (mots du sujet et du corps), Sujet… (mots du sujet seul), Date conversation… (aujourd'hui, 7 derniers jours, 30 derniers jours, cette année, plus d'un an, ou après / avant une date tapée, ou depuis / il y a plus de une durée tapée), Date mail… (les mêmes choix).

**Deux critères de date** (ajouté en novembre 2026). **Date conversation…** porte sur la date de la conversation, celle de son dernier mail (sans ceux de Deleted, Junk et Banned, comme la date affichée dans la liste) : « après le 2020-01-01 » garde les conversations encore actives depuis 2020, « avant le 2020-01-01 » celles qui se sont tues avant. C'est le critère de presque toutes les recherches, et il ne coûte rien : la date est celle sur laquelle la liste est triée. **Date mail…** porte sur chaque mail : une conversation qui court de 2016 à 2026 est trouvée par « avant le 2020-01-01 », parce que certains de ses mails sont de 2016. Elle reste triée et datée par son dernier mail. Ce critère regarde les mails des dossiers listés, comme De… (« Négation ») ; il est plus lourd (son coût suit le nombre de mails de l'intervalle), d'où un critère à part plutôt qu'un changement du premier. Chaque critère a ses deux bornes et ses pastilles ; celles de Date mail… sont préfixées (« mail : 7 derniers jours »). Les deux se combinent.

La ligne de saisie de chaque panneau de valeur dit ce qu'on peut taper tant que rien n'est tapé : les panneaux de date (Date conversation… et Date mail…, les mêmes ; « Date… » ci-dessous vaut pour les deux) s'ouvrent sur leurs préréglages, et une aide montrée seulement quand le panneau est vide restait invisible. Date… accepte `2026-10-01`, `1/10/2026`, `1/10/26`, et **six chiffres**, lus en `jjmmaa` et en `aammjj`. Chaque lecture qui donne un vrai jour propose « après le » et « avant le » ce jour : une seule lecture valide donne deux lignes (`311299` : 31 décembre 1999), deux lectures valides en donnent quatre, et chaque ligne dit à droite sa lecture (« jj mm aa », « aa mm jj ») : `010120` est le 1er janvier 2020 ou le 20 janvier 2001. Deux lectures qui tombent le même jour comptent une fois (`111111`). Une année sur deux chiffres sous 70 est 20aa, à partir de 70 19aa (pas de mail avant 1970).

Date… accepte aussi une **durée** : un nombre et une unité, par sa première lettre en français ou en anglais, ou son mot entier : `j`/`d` jour, `s`/`w` semaine, `m` mois, `a`/`y` année (`10j`, `3 s`, `8w`, `6m`, `2 ans`). Les deux langues sont toujours comprises, les lettres ne se chevauchant pas. Sans nombre, c'est 1 (`m` : un mois). Le panneau propose « depuis 10 jours » et « il y a plus de 10 jours », avec à droite la date obtenue. Toutes les unités se comptent depuis maintenant selon le calendrier : `1j` est hier à la même heure, `1m` le même jour du mois précédent, donc `7j` et `1s` coïncident.

Les deux bornes de date sont indépendantes et forment un intervalle : chaque ligne fixe la borne basse (« après », « depuis », les préréglages sauf le dernier) ou la borne haute (« avant », « il y a plus de », « plus d'un an ») et garde l'autre. « après le 2020-01-01 » puis « avant le 2020-07-01 » donne le premier semestre 2020, en deux pastilles supprimables séparément. Rechoisir une borne la remplace. Une ligne qui ne laisserait aucun jour entre les deux bornes l'indique à droite (« rien entre les deux ») sans être refusée.

Texte… ne cherche pas dans les adresses : chercher « dupont » dans le texte ne doit pas ramener tous les mails de Dupont, c'est le rôle de De… et À…. Seule la recherche `/` (« Chercher partout ») reste le joker qui cherche aussi dans l'expéditeur et les destinataires.

#### Accès au clavier

Le sélecteur s'ouvre sur sa première ligne et entrée choisit la ligne en surbrillance : ce qui compte est la ligne qui arrive en tête quand on tape. Les lignes sont classées, sans tenir compte de la casse ni des accents, et à rang égal gardent l'ordre de la liste :

1. le libellé commence par la frappe (`lu` → « lus », avant « non lus » qui ne fait que le contenir) ;
2. chaque mot tapé commence un mot du libellé, dans n'importe quel ordre (`jointe sans` → « sans pièce jointe ») ;
3. les lettres tapées sont les initiales des mots du libellé, pour un seul mot tapé d'au moins deux lettres (`nl` → « non lus », `sd` → « sans drapeau », `spj` → « sans pièce jointe », `sm` → « sans ma réponse ») ;
4. chaque mot tapé apparaît n'importe où dans le libellé.

Les attributs à valeur ont aussi des mots de recherche cachés, traduits : `exp`, `expéditeur` trouvent De… ; `cc`, `copie`, `destinataire` trouvent À… ; `texte`, `mots`, `corps` trouvent Texte… ; `titre`, `objet` trouvent Sujet… ; `quand`, `jour`, `période` trouvent Date conversation… ; `message` trouve Date mail…. Les libellés français sont choisis pour que deux lettres suffisent toujours : `de` De…, `da` Date conversation… (le critère courant vient avant Date mail…), `dm` Date mail…, `dc` Date conversation…, `dr` drapeau, `a` À…, `l` lus, `no` non lus, `pi` ou `pj` pièce jointe, `r` répondus, `co` conversations, `m` mails seuls, `te` Texte…, `su` Sujet…. « À… » remplace « Destinataire… », qui commençait comme « De… », et reprend l'en-tête « À : » du mail et la pastille « à : ». Ce classement vaut pour tous les sélecteurs (`@`, `l`, `t`, `g`, `f`).

#### Mots avec ponctuation

Dans `/`, Texte… et Sujet…, chaque mot tapé est cherché comme début de mot, sans tenir compte des accents ni de la casse. Un mot qui contient un signe de ponctuation (`.` `@` `/` `:` `_`…) est en plus cherché **tel qu'il est écrit** : `paris.fr` trouve « paris.fr », « contact@paris.fr », « mairie-paris.fr », mais plus « 75016 PARIS / France » ni « Paris FR » (ajouté en novembre 2026). L'index plein texte découpe les mots sur la ponctuation et lisait `paris.fr` comme « paris » suivi d'un mot commençant par « fr » : sur une vraie boîte, 14 542 mails au lieu des 417 qui contiennent « paris.fr ». Le tiret et l'apostrophe ne comptent pas : ils lient les mots de la prose (`rendez-vous` trouve « rendez vous », `l'eau` trouve « l’eau »). Les accents ne sont pas ignorés dans la partie écrite telle quelle (`eleve.fr` ne trouve pas « élève.fr »). Ce contrôle lit le texte des mails candidats : environ 0,4 s par page sur 400 000 mails, pour ces mots seulement. Un mot avec ponctuation précédé d'un tiret est exclu de la même façon.

#### Négation

Chaque attribut oui / non a son contraire en ligne propre (« sans drapeau », « non lus », « sans ma réponse »…), atteint par ses initiales ; choisir le contraire d'une valeur active la remplace.

Pour De…, À…, Texte… et Sujet…, la négation se choisit dans le panneau de la valeur, sans doubler les lignes du sélecteur :

- la frappe propose deux lignes : `+ acme` (entrée) et `− pas acme` (flèche bas puis entrée) ; taper `-acme` (tiret devant) ne laisse que l'exclusion, et les contacts suggérés sont alors proposés en exclusion ;
- une valeur est voulue ou exclue, pas les deux : le dernier choix l'emporte ;
- la pastille le dit : « de : acme », « pas de : acme », « à : acme », « pas à : acme » ; Texte… et Sujet… gardent le tiret des mots exclus (« texte : renouvellement -facture ») ;
- les valeurs positives d'un attribut se combinent par « ou », ses valeurs exclues par « et » : « de : alice », « de : bob », « pas de : @acme.fr » garde les mails d'Alice ou de Bob qui ne viennent pas d'acme.fr. Une conversation est exclue dès qu'un de ses mails des dossiers listés correspond à une valeur exclue. Les dossiers listés, et pas seulement celui où la conversation s'affiche : dans Everything, une conversation de l'Inbox s'affiche dans l'Inbox, et « pas de : @acme.fr » doit voir le mail d'acme.fr qu'elle a encore dans Archive. La règle vaut pour De…, À…, pièce jointe et Date mail… ;
- **domaine** : une valeur qui commence par `@` désigne un domaine exact et ses sous-domaines (`@acme.fr` couvre `jean@acme.fr` et `x@mail.acme.fr`, pas `x@notacme.fr`). Sans `@`, la valeur reste une partie de nom ou d'adresse ;
- dans Texte… et Sujet…, `-mot` exclut un mot parmi d'autres (`renouvellement -facture`). Choisir de nouveau Texte… ou Sujet… ouvre le panneau sur les mots actuels, sélectionnés : taper (ou coller) les remplace, entrée seule les garde, une flèche lève la sélection pour les éditer (ajouter une exclusion décidée après coup : flèche droite, puis ` -facture`), effacer puis entrée retire le critère. Auparavant les nouveaux mots s'ajoutaient aux anciens, invisibles dans le panneau : une seconde recherche par sujet restreignait la première au lieu de la remplacer ;
- `/` accepte aussi `-mot`, cherché partout, adresses comprises.

Exemple, la gestion des registrars : trouver les mails qui parlent du domaine acme.fr sans ceux envoyés ou reçus par ce domaine, c'est Texte… `acme.fr`, puis De… `-@acme.fr`, puis À… `-@acme.fr` ; trois pastilles « texte : acme.fr », « pas de : @acme.fr », « pas à : @acme.fr ».

Les attributs avec « … » agrandissent aussitôt le panneau pour choisir la valeur. Choisir de nouveau un attribut actif le retire. Les attributs actifs s'affichent en pastilles dans la barre de statut ; un clic sur une pastille la retire, échap les retire toutes ; ils restent quand on change de dossier ou de compte.

`/` est le raccourci de la recherche partout : il ouvre un panneau intitulé « Chercher partout » ; entrée va dans **All Mail** de la vue (tous les dossiers sauf Deleted, Junk et Banned, tous les comptes dans « all ») avec ces mots ; tapé depuis Everything, il reste dans Everything. Le résultat est la liste ordinaire, une ligne par conversation avec son dossier, la pastille et `f` pour affiner. Échap retire les attributs et revient au dossier d'où `/` a été tapé ; si l'on est allé ailleurs entre-temps (`;`, `@`, clic sur un dossier), échap retire seulement les attributs. Un second `/` remplace les mots et garde le dossier de retour. `/` reste le joker le plus large : ses mots sont cherchés partout, sujet, expéditeur, destinataires et texte (pastille « partout : … »), là où Texte… laisse les adresses de côté.

### Dossiers virtuels

Trois dossiers virtuels réunissent des dossiers de la vue (de tous les comptes dans « all »). Chacun termine le bloc de ce qu'il réunit dans le menu `;` et la colonne des dossiers, et s'y distingue des vrais dossiers : une icône de pile devant son nom, et la couleur d'accent (la police à chasse fixe du thème n'a pas d'italique). Réorganisé en novembre 2026 : Work ajouté, All Mail passé de `E` à `m`.

- **Work** (`;o`) : Inbox, Todo et Waiting ensemble, ce qui reste à traiter. Chaque ligne affiche son dossier. Ses critères (De…, À…, pièce jointe, Date mail…) ne regardent que les mails de ces trois dossiers.
- **All Mail** (`;m`) : tous les dossiers sauf Deleted, Junk et Banned, Quarantine comprise. C'est ce que veulent la plupart des recherches : ce qu'on a supprimé ou rejeté n'y encombre pas les résultats, et le mail d'un expéditeur pas encore accepté s'y trouve. Le nom est celui de Gmail, pour le même sens (tout sauf la corbeille et le spam). Une conversation dont le dernier mail a été supprimé y garde sa place et sa date, celles de son dernier mail resté dans All Mail ; ses critères (De…, À…, pièce jointe, Date mail…) ne regardent que les mails de ces dossiers. Ce sont les conversations dont le dossier n'est ni Deleted, ni Junk, ni Banned (`dossiers.md`, « Dossier d'une conversation ») : une conversation rejetée n'y revient pas par la réponse de l'utilisateur restée dans Sent.
- **Everything** (`;e`) : vraiment tous les dossiers.

Une conversation présente dans plusieurs dossiers (Inbox et Sent) n'y figure qu'une fois, dans son dossier (`dossiers.md`, « Dossier d'une conversation ») : l'Inbox, et non Sent, quand le dernier mail est la réponse de l'utilisateur. Comme la liste mélange tous les dossiers, chaque ligne affiche son dossier (colonne discrète avant le nombre de messages), et le lecteur l'affiche à l'extrémité droite de la ligne du sujet. Dans « all », le compte suit le dossier (« Archive · perso ») : toujours dans le lecteur, et dans la liste seulement si le sujet garde au moins 30 caractères ; sinon le dossier seul.

Il y a deux contextes totalement différents dans l'application : 
- "navigation" : la gestion du stock de mail d'une part. lecture, recherche, screening, déplacement, etc
- "edition" : L'écriture d'un nouveau mail (from scratch ou en réponse) d'autre part. 

Les raccourcis clavier sont différents en fonction de ces deux contextes. 

Voici la liste des principaux raccourcis clavier dans le contexte "navigation"


hjkl : naviguer dans les mails
j et k : mail suivant et précédent
ctrl-d et ctrl-u : une demi-page plus bas ou plus haut dans la liste des mails, comme dans Zed ; le curseur avance de la moitié des lignes visibles et s'arrête au premier ou au dernier mail
h et l : change de "colonne"
	Si on est focus sur la liste des mails, h déclenche l'écran pour passer à un autre dossier. Dans l'interface classique, c'est la barre à gauche. Dans l'interface geek, ce rôle est tenu par `;` : `h` n'y fait rien. 
	Et depuis la liste des mails, l déclenche l'écran de "link".

Avec une majuscule (JK), ou maj + flèche bas / haut, on passe au mail suivant et précédent en conservant le mail actuel sélectionné. On peut en effet faire des sélections multiples pour déplacer ou taguer plusieurs mails en même temps.  

À la souris (thème geek) : un clic sélectionne un mail et efface les marques ; ⌘-clic marque ou démarque un mail ; ⇧-clic marque tous les mails entre le mail courant et le mail cliqué. Dans les écrans contacts, tags et groupes, un double-clic équivaut à entrée. Un clic agit immédiatement, sans attendre un éventuel double-clic.


Les opérations classiques sur un mail sont (elles s'appliquent aux mails marqués avec J/K, sinon au mail sélectionné) : 
r répondre à tous (`reponse.md`) : contexte édition, ⌘↩ envoie, ⇧⌘↩ choisit où ranger la conversation puis envoie, échap garde le brouillon, ⌘⌫ l'abandonne ; ⇥ et ⇧⇥ passent de champ en champ (À, Cc, Cci, Objet, texte), ⌥⌘B ouvre la ligne Cci, ⌥⌘F choisit l'adresse (donc le compte) d'envoi
R répondre à l'expéditeur seul (`reponse.md`, section 4 bis)
F forwarder : transférer le mail et ses fichiers
c écrire un nouveau mail
d mark done (archiver)
m move (déplacer), puis choisir le dossier. mt déplacer dans le dossier Todo, md déplacer dans le dossier Deleted. Le menu ne propose que les cibles permises depuis le dossier du mail. En Quarantine, la lettre seule suffit (`i`, `n`, `p`, `x`, `b`, `d`) : voir « Quarantine ».
del supprimer (déplacer dans Deleted)
t tag (ajouter un tag ; si tous les mails visés l'ont déjà, le retirer)
g group (ajoute l'expéditeur dans un groupe)
l link (panneau des liens du mail ; entrée ouvre le lien dans le navigateur). Dans un mail de code (code de vérification à usage unique envoyé par un site ou une application : « Votre code de vérification : 610731 »), la toute première ligne est le **code de vérification**, avec une icône de cadenas : entrée (ou ⌘-entrée) le copie dans le presse-papiers, la barre d'état le confirme. Dans une conversation, c'est le code du dernier message qui en donne un. Le noyau reconnaît le code (règles dans `docs/core-library.md`, « Verification codes ») ; une conversation entre personnes (réponse, transfert) n'est pas un mail de code. Ensuite, « Lien vers ce mail » : entrée copie un lien Markdown `[sujet](nicemailer://m/<id>)` vers le mail, pour le coller dans une note (Obsidian), ⌘-entrée l'URL seule ; un clic sur ce lien ouvre le mail dans NiceMailer. Un même lien (même destination, même derrière des adresses de suivi différentes) n'apparaît qu'une fois, avec son premier libellé suivi du nombre exact d'autres libellés qui y mènent : « ressent "les angoisses" du pays (et 2 autres) ». Des liens collés l'un à l'autre vers la même destination forment un seul lien, leurs textes réunis. D'abord les liens qui ont un texte (« Voir ma facture »), puis les numéros de téléphone trouvés dans le mail, avec une icône de téléphone (entrée lance l'appel dans l'app Téléphone du Mac), puis les pièces jointes, enfin les liens sans texte (le plus souvent une image menant à un réseau social). Le panneau est aussi large que la fenêtre le permet, pour lire les liens longs.
u lu / non lu
U marque lu, quel que soit l'état actuel (un mail lu reste lu, contrairement à u)
z annule le dernier déplacement ou la dernière décision (les mails reviennent, la décision précédente aussi, et le curseur revient sur la conversation rétablie) ; chaque z remonte d'une étape
\* drapeau
/ chercher partout : des mots n'importe où, adresses comprises, dans All Mail (voir le sélecteur d'attributs) ; échap revient au dossier de départ
? liste des touches, regroupées par usage (naviguer, lire, sélectionner, agir sur les mails, écrire, chercher et aller, fenêtre), une touche par ligne, dans un panneau centré de trois colonnes au plus, équilibrées en hauteur ; moins de colonnes si la fenêtre est étroite. Les commandes pas encore disponibles sont grisées. N'importe quelle touche ou un clic à côté la ferme
échap annule un préfixe, ferme un panneau, démarque, puis revient du tag, groupe ou contact listé au dossier
⌃⌘F met la fenêtre en plein écran et l'en sort, partout (liste, menu, réponse en cours d'écriture), que la barre de titre soit masquée ou non
en une colonne, sur la liste, espace ouvre le mail comme entrée ; mail affiché, espace et ctrl-f le font défiler d'une page vers le bas, maj-espace et ctrl-b vers le haut, en texte comme en HTML

Le prefix ";" permet de changer d’écran
; aller à un dossier. Le menu (et la colonne des dossiers) les range en quatre blocs, séparés par une ligne vide, chacun terminé par son dossier virtuel (« Dossiers virtuels ») : ce qui est à traiter, puis Work ; ce qui est classé, Quarantine comprise (son mail attend une décision sur l'expéditeur, pas un travail) ; ce qui a été écrit, puis All Mail ; ce qui a été jeté, puis Everything. Viennent ensuite les écrans Contacts, Tags, Groups, et History à part. Les dossiers propres à l'utilisateur forment un bloc après les dossiers classés. Le nombre de mails non lus de chaque dossier est aligné à l'extrémité droite de sa ligne.

i Inbox
t Todo
w Waiting
o Work

a Archive
n Newsletter
p Paperwork
q Quarantine

s Sent
r Drafts
m All Mail

d Deleted
x Junk
b Banned
e Everything

c Contacts
T Tags
g Groups

h History

Les lettres sont celles des dossiers (`dossiers.md`) : `j` est réservé à la navigation, d'où `x` pour Junk, y compris après un préfixe.

ainsi ;i va à l’inbox, ;p va à Paperwork, ;t va à Todo, etc

Le prefix ";" permet également d'aller aux autres écrans : 
c Contact (liste des adresses ; / filtre ; m choisit la décision sur l'adresse, M sur son domaine : i Inbox, n Newsletter, p Paperwork, b Banned, - aucune ; entrée liste les mails du contact, tous dossiers)
h History (affichage de l'historique issu de l audit log, plus récent d'abord)
T tag (majuscule : `;t` est Todo ; entrée liste les mails du tag, et des tags de même nom des autres comptes)
g group (entrée liste les mails des membres du groupe)

Sur ces écrans, j/k se déplacent et échap revient aux mails.


Il y a deux autres préfixes : s et v. (`d` était prévu pour display, mais `d` seul archive : une touche ne peut pas être à la fois une action et un préfixe.)
s save (dans ~/Downloads ; un fichier existant n'est jamais écrasé ; s'applique aux mails marqués, ou à défaut au mail sous le curseur)
	c to clipboard (en première ligne) : copie dans le presse-papiers le texte du dernier mail de chaque conversation, comme t (en-têtes From, To, Cc, Date, Subject, puis le corps), dans l'ordre de la liste, une ligne vide entre deux mails. Les paragraphes doivent rester exploitables une fois collés dans un document : un paragraphe tient sur une seule ligne, sans les retours à la ligne que l'app de l'expéditeur insère tous les 72 caractères (format=flowed, texte coupé à largeur fixe), et une seule ligne vide sépare deux paragraphes. Les lignes voulues restent telles quelles : citations (`>`, par profondeur), signature (après `-- `), listes, lignes indentées, tableaux, lignes courtes (formule de politesse, nom), suite de phrases une par ligne. La barre de statut le confirme : « Devis toiture » copié dans le presse-papiers ; pour plusieurs mails, leur nombre.
	a all attachments : les pièces jointes des mails marqués, ou à défaut du mail sous le curseur ; tous les messages de chaque conversation ; un fichier joint de nouveau à une réponse (même nom, type et taille) n'est enregistré qu'une fois
	g attachments but graphics (gif, png, ...) : de même, sans les images (logos, signatures, photos)
	p as pdf (pas encore)
	e as eml file (dernier mail de chaque conversation, comme t et m)
	t as text file (T : enregistre puis ouvre le fichier au premier plan, dans l'app associée par macOS)
	m as markdown file (M : de même)
	Dans le menu, la ligne de t (et de m) rappelle à droite, en grisé, « T : et ouvrir ». Avec les flèches, ⌘-entrée sur la ligne fait de même. Au-delà de 5 mails, les fichiers sont enregistrés mais pas ouverts, et la barre de statut le dit.
v display (view)
	1 une seule colonne (masque la liste des dossiers et la lecture d'un mail). Entrée, ou un double-clic sur une ligne, affiche le mail dans toute la fenêtre et le marque lu aussitôt (j / k passent au mail suivant / précédent) ; Escape revient à la liste
	2 colonnes (presentation classique : liste des mails à gauche et lecture du mail à droite)
	3 colonnes (liste des dossiers à gauche, puis liste des mails au milieu et lecture du mail à droite)
	c creation date (mail trié par date de création, ou de réception)
	m modification date (mail trié par date de modification: permet de voir ce que l'on a modifié)
	h texte / HTML (choix mémorisé pour l'expéditeur)
	i images distantes (choix mémorisé pour l'expéditeur, appliqué aux seuls mails authentifiés)
	s barre de statut : en bas, en haut, masquée

Le nombre de colonnes, la position de la barre de statut et la largeur de la liste sont mémorisés.

Taille du texte, comme dans Zed : ⌘+ (ou ⌘=, sans Maj sur un clavier AZERTY) agrandit toute l'interface, ⌘− la réduit, ⌘0 revient à la taille par défaut, d'un point à la fois entre 9 et 24 points (12 par défaut). Une seule taille de base : polices, hauteur des lignes, largeur des colonnes, menus, sélecteurs et vue HTML en découlent. Elle est mémorisée.
