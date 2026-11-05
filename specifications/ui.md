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
- Corps des mails en texte par défaut. Un sélecteur `text | html | images` en haut du mail, et `vh`, basculent en HTML assaini ; `images` (ou `vi`) charge aussi les images distantes (voir « Images distantes »). Le choix est mémorisé **par adresse d'expéditeur** (`address.body_format`, `address.remote_images`, `schema.sql`).
- Compte unifié **all** : avec plusieurs comptes, l'app s'ouvre sur « all », qui regroupe tous les comptes. Chaque dossier (Inbox, Archive…) y réunit les dossiers de même rôle de tous les comptes, avec leurs compteurs additionnés. La colonne des dossiers n'a qu'un bloc, intitulé « all » ou le nom du compte choisi ; le sélecteur de dossiers et le menu `;` ne montrent que les dossiers de cette vue, sans notion de compte.
- Barre de statut : le mode (`NOR`, ou le préfixe en attente, comme dans Helix), puis le compte (icône @, « all » pour la vue unifiée) et le dossier (icône de dossier), chacun suivi du rappel de sa touche. Chaque élément s'ouvre d'une touche ou d'un clic : le mode avec `?` (liste des touches), le compte avec `@` (sélecteur de comptes, « all » en tête : on arrive dans le dossier de même rôle, sinon l'Inbox), le dossier avec `;` (menu des dossiers de la vue, d'« Everything » et des écrans, qui se déroule à gauche depuis le bloc dossier de la barre de statut : en dessous si la barre est en haut, au-dessus si elle est en bas ; ses lignes se cliquent aussi). Les menus `m` (déplacer), `s` (enregistrer) et `v` (affichage) ont aussi leur bloc dans la barre de statut, une icône suivie de la touche. Un large espace sépare trois groupes : ce que la liste affiche (`@`, `;`, `F` et les pastilles), la préférence d'affichage (`v`), les actions sur le mail courant (`m`, `s`). Leur menu se déroule depuis ce bloc de la même façon. Dans un menu, les flèches surlignent une ligne (bas commence à la première, haut à la dernière, la liste tourne) et entrée va à la ligne surlignée ; une touche agit toujours directement. Barre masquée : les menus s'affichent en bas à droite. `h` ne fait plus rien : il faisait double emploi avec `;`. Pendant qu'un tag, un groupe ou un contact est listé, la barre l'affiche à la place. Les messages (résultat d'une action, erreur) s'affichent à droite et disparaissent à la touche suivante. Un déplacement nomme le mail et le dossier : « Devis toiture » déplacé dans Archive (sujet coupé à 50 caractères) ; pour plusieurs mails, leur nombre : 3 conversations déplacées dans Archive.
- Les sélecteurs (`@`, `l`, `t`, `g`, `F`) sont des listes filtrables à la manière de Helix : on tape pour filtrer (chaque mot tapé doit apparaître), `ctrl-n` / `ctrl-p` ou les flèches pour se déplacer, entrée pour choisir, échap pour fermer. À l'ouverture, la première ligne est sélectionnée ; la liste tourne : flèche haut (ou `ctrl-p`) depuis la première ligne sélectionne la dernière, flèche bas (ou `ctrl-n`) depuis la dernière revient à la première. Pour `t` et `g`, taper un nom absent propose de le créer. Le texte tapé s'édite comme dans tout champ macOS : flèches gauche et droite pour déplacer le curseur (⌥ par mot, ⌘ jusqu'au bout), effacement des deux côtés du curseur, et ⌘V colle le texte copié dans une autre app, sur une seule ligne, sans les espaces autour. Un clic en dehors d'un sélecteur, d'un menu ou de l'aide le ferme, comme échap, sans rien faire d'autre.

L'interface utilisateur de NiceMailer est, quel que soit le thème choisi, minimaliste et sobre. 

La fenêtre principale permet de lister les mails en deux colonnes : 
- A gauche, sur un petit tiers de l'écran, mais resizable, la liste des mails. 
- A droite, sur le reste de l'écran, l'affichage du mail sélectionné à gauche. 

Lors de l'ouverture d'une fenêtre, le premier mail est systématiquement sélectionné dans la liste, de sorte que la partie de droite affiche quelque chose. 

### Affichage d'une conversation

L'affichage est simplifié au maximum : chaque message montre ce qu'il apporte. Le noyau découpe chaque message et l'app n'affiche que le résultat (détail : `docs/thread-display.md`).

- Masqués par défaut : la citation finale (historique cité, avec sa ligne « … a écrit : » ou le bloc d'en-têtes d'Outlook), le texte qui recopie un message précédent, les signatures (après `-- `, ou lignes finales répétées par un expéditeur), les mentions (« Envoyé de mon iPhone », avis de confidentialité). Chaque partie masquée est remplacée par une ligne « ··· » cliquable.
- Les citations intercalées entre les réponses restent visibles, atténuées, avec une barre par niveau. Un message transféré n'est jamais masqué.
- Comme dans Gmail, le dernier message et les non-lus sont dépliés ; les autres tiennent sur une ligne (expéditeur, date, début du texte). Un message qui ne fait que citer l'indique.
- En-têtes allégés : expéditeur et date ; les destinataires seulement s'ils changent d'un message à l'autre.
- `T` (comme dans mutt) affiche ou masque tout ce que la vue simplifiée cache, pour la conversation ouverte ; `S` va au texte qui suit la citation suivante. Un clic sur un en-tête plie ou déplie le message, un clic sur « ··· » affiche cette partie.

### Pièces jointes

- Au pied de chaque mail déplié, une dalle par pièce jointe : l'icône de son type (SF Symbol, aux couleurs du thème : image, PDF, texte, tableur, présentation, archive, audio, vidéo, code, agenda, contact, mail, fichier), le nom coupé au milieu pour garder l'extension, la taille. 3 à 5 dalles par ligne selon la largeur du panneau. Un mail plié montre un trombone et le nombre de pièces jointes.
- Un clic enregistre le fichier dans Téléchargements, sans jamais écraser un fichier existant (« devis (2).pdf ») ; la barre de statut le confirme. ⌘-clic l'enregistre de même, puis ouvre la copie enregistrée avec l'app par défaut, au premier plan.
- Les images affichées par le HTML du mail (`cid:`) ne sont pas des dalles. Une image marquée « intégrée » qu'aucun HTML n'affiche (photo d'Apple Mail dans un mail texte) en est une.
- Au clavier, `l` liste les liens du mail puis une ligne par fichier joint, avec son icône ; entrée enregistre le fichier, ⌘-entrée l'enregistre puis ouvre aussitôt la copie enregistrée, au premier plan.
- En HTML, les mêmes dalles suivent le corps du mail.

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
- Dans une liste filtrée (non lus, avec drapeau…), le mail sélectionné qui ne correspond plus (lu après une seconde, `u`, `*`) reste sous le curseur, affiché avec son nouvel état, jusqu'à ce qu'on le quitte ; il sort alors de la liste. Sans cela, une liste de mails non lus se vidait toute seule, un mail par seconde.

### Sélecteur d'attributs

À droite du compte et du dossier, dans la barre de statut, une loupe (touche `F`, ou un clic) ouvre le sélecteur d'attributs, qui affine la liste courante (dossier d'un compte, ou de « all », ou mails d'un tag, d'un groupe ou d'un contact, dont la barre de statut garde alors « tag : … » suivi de la loupe et des pastilles) ; les attributs se combinent par « et ».

- non lus / lus, drapeau / sans drapeau, pièce jointe / sans pièce jointe, répondus / sans ma réponse (répondu par l'utilisateur), conversations / mails seuls ;
- De…, À… (À et Cc ; une partie d'un nom ou d'une adresse, contacts suggérés pendant la frappe ; plusieurs valeurs combinées par « ou » ; le nom est celui du contact, `address.name`, et non le nom d'affichage de chaque mail : un mail envoyé par une adresse partagée, comme `notifications@github.com` au nom de Thomas, n'est pas « de : thomas »), Texte… (mots du sujet et du corps), Sujet… (mots du sujet seul), Date… (aujourd'hui, 7 derniers jours, 30 derniers jours, cette année, plus d'un an, ou après / avant une date tapée, ou depuis / il y a plus de une durée tapée).

La ligne de saisie de chaque panneau de valeur dit ce qu'on peut taper tant que rien n'est tapé : Date… s'ouvre sur ses préréglages, et une aide montrée seulement quand le panneau est vide restait invisible. Date… accepte `2026-10-01`, `1/10/2026`, `1/10/26`, et **six chiffres**, lus en `jjmmaa` et en `aammjj`. Chaque lecture qui donne un vrai jour propose « après le » et « avant le » ce jour : une seule lecture valide donne deux lignes (`311299` : 31 décembre 1999), deux lectures valides en donnent quatre, et chaque ligne dit à droite sa lecture (« jj mm aa », « aa mm jj ») : `010120` est le 1er janvier 2020 ou le 20 janvier 2001. Deux lectures qui tombent le même jour comptent une fois (`111111`). Une année sur deux chiffres sous 70 est 20aa, à partir de 70 19aa (pas de mail avant 1970).

Date… accepte aussi une **durée** : un nombre et une unité, par sa première lettre en français ou en anglais, ou son mot entier : `j`/`d` jour, `s`/`w` semaine, `m` mois, `a`/`y` année (`10j`, `3 s`, `8w`, `6m`, `2 ans`). Les deux langues sont toujours comprises, les lettres ne se chevauchant pas. Sans nombre, c'est 1 (`m` : un mois). Le panneau propose « depuis 10 jours » et « il y a plus de 10 jours », avec à droite la date obtenue. Toutes les unités se comptent depuis maintenant selon le calendrier : `1j` est hier à la même heure, `1m` le même jour du mois précédent, donc `7j` et `1s` coïncident.

Les deux bornes de date sont indépendantes et forment un intervalle : chaque ligne fixe la borne basse (« après », « depuis », les préréglages sauf le dernier) ou la borne haute (« avant », « il y a plus de », « plus d'un an ») et garde l'autre. « après le 2020-01-01 » puis « avant le 2020-07-01 » donne le premier semestre 2020, en deux pastilles supprimables séparément. Rechoisir une borne la remplace. Une ligne qui ne laisserait aucun jour entre les deux bornes l'indique à droite (« rien entre les deux ») sans être refusée.

Texte… ne cherche pas dans les adresses : chercher « dupont » dans le texte ne doit pas ramener tous les mails de Dupont, c'est le rôle de De… et À…. Seule la recherche `/` (« Chercher partout ») reste le joker qui cherche aussi dans l'expéditeur et les destinataires.

#### Accès au clavier

Le sélecteur s'ouvre sur sa première ligne et entrée choisit la ligne en surbrillance : ce qui compte est la ligne qui arrive en tête quand on tape. Les lignes sont classées, sans tenir compte de la casse ni des accents, et à rang égal gardent l'ordre de la liste :

1. le libellé commence par la frappe (`lu` → « lus », avant « non lus » qui ne fait que le contenir) ;
2. chaque mot tapé commence un mot du libellé, dans n'importe quel ordre (`jointe sans` → « sans pièce jointe ») ;
3. les lettres tapées sont les initiales des mots du libellé, pour un seul mot tapé d'au moins deux lettres (`nl` → « non lus », `sd` → « sans drapeau », `spj` → « sans pièce jointe », `sm` → « sans ma réponse ») ;
4. chaque mot tapé apparaît n'importe où dans le libellé.

Les attributs à valeur ont aussi des mots de recherche cachés, traduits : `exp`, `expéditeur` trouvent De… ; `cc`, `copie`, `destinataire` trouvent À… ; `texte`, `mots`, `corps` trouvent Texte… ; `titre`, `objet` trouvent Sujet… ; `quand`, `jour`, `période` trouvent Date…. Les libellés français sont choisis pour que deux lettres suffisent toujours : `de` De…, `da` Date…, `dr` drapeau, `a` À…, `l` lus, `no` non lus, `pi` ou `pj` pièce jointe, `r` répondus, `co` conversations, `m` mails seuls, `te` Texte…, `su` Sujet…. « À… » remplace « Destinataire… », qui commençait comme « De… », et reprend l'en-tête « À : » du mail et la pastille « à : ». Ce classement vaut pour tous les sélecteurs (`@`, `l`, `t`, `g`, `F`).

#### Négation

Chaque attribut oui / non a son contraire en ligne propre (« sans drapeau », « non lus », « sans ma réponse »…), atteint par ses initiales ; choisir le contraire d'une valeur active la remplace.

Pour De…, À…, Texte… et Sujet…, la négation se choisit dans le panneau de la valeur, sans doubler les lignes du sélecteur :

- la frappe propose deux lignes : `+ acme` (entrée) et `− pas acme` (flèche bas puis entrée) ; taper `-acme` (tiret devant) ne laisse que l'exclusion, et les contacts suggérés sont alors proposés en exclusion ;
- une valeur est voulue ou exclue, pas les deux : le dernier choix l'emporte ;
- la pastille le dit : « de : acme », « pas de : acme », « à : acme », « pas à : acme » ; Texte… et Sujet… gardent le tiret des mots exclus (« texte : renouvellement -facture ») ;
- les valeurs positives d'un attribut se combinent par « ou », ses valeurs exclues par « et » : « de : alice », « de : bob », « pas de : @acme.fr » garde les mails d'Alice ou de Bob qui ne viennent pas d'acme.fr. Une conversation est exclue dès qu'un de ses mails des dossiers listés correspond à une valeur exclue. Les dossiers listés, et pas seulement celui où la conversation s'affiche : dans Everything, une conversation dont le dernier mail est la réponse de l'utilisateur s'affiche dans Sent, et « pas de : @acme.fr » doit voir le mail d'acme.fr resté dans l'Inbox. La règle vaut pour De…, À… et pièce jointe ;
- **domaine** : une valeur qui commence par `@` désigne un domaine exact et ses sous-domaines (`@acme.fr` couvre `jean@acme.fr` et `x@mail.acme.fr`, pas `x@notacme.fr`). Sans `@`, la valeur reste une partie de nom ou d'adresse ;
- dans Texte… et Sujet…, `-mot` exclut un mot parmi d'autres (`renouvellement -facture`). Choisir de nouveau Texte… ou Sujet… ouvre le panneau sur les mots actuels, sélectionnés : taper (ou coller) les remplace, entrée seule les garde, une flèche lève la sélection pour les éditer (ajouter une exclusion décidée après coup : flèche droite, puis ` -facture`), effacer puis entrée retire le critère. Auparavant les nouveaux mots s'ajoutaient aux anciens, invisibles dans le panneau : une seconde recherche par sujet restreignait la première au lieu de la remplacer ;
- `/` accepte aussi `-mot`, cherché partout, adresses comprises.

Exemple, la gestion des registrars : trouver les mails qui parlent du domaine acme.fr sans ceux envoyés ou reçus par ce domaine, c'est Texte… `acme.fr`, puis De… `-@acme.fr`, puis À… `-@acme.fr` ; trois pastilles « texte : acme.fr », « pas de : @acme.fr », « pas à : @acme.fr ».

Les attributs avec « … » agrandissent aussitôt le panneau pour choisir la valeur. Choisir de nouveau un attribut actif le retire. Les attributs actifs s'affichent en pastilles dans la barre de statut ; un clic sur une pastille la retire, échap les retire toutes ; ils restent quand on change de dossier ou de compte.

`/` est le raccourci de la recherche partout : il ouvre un panneau intitulé « Chercher partout » ; entrée va dans **Everything** de la vue (tous les dossiers, tous les comptes dans « all ») avec ces mots. Le résultat est la liste ordinaire, une ligne par conversation avec son dossier, la pastille et `F` pour affiner. Échap retire les attributs et revient au dossier d'où `/` a été tapé ; si l'on est allé ailleurs entre-temps (`;`, `@`, clic sur un dossier), échap retire seulement les attributs. Un second `/` remplace les mots et garde le dossier de retour. `/` reste le joker le plus large : ses mots sont cherchés partout, sujet, expéditeur, destinataires et texte (pastille « partout : … »), là où Texte… laisse les adresses de côté.

Au pied de la liste des dossiers, un dossier virtuel **Everything** (`;e`) réunit tous les dossiers de la vue (de tous les comptes dans « all ») pour chercher partout avec les attributs. Une conversation présente dans plusieurs dossiers (Inbox et Sent) n'y figure qu'une fois, dans le dossier de son dernier mail. Comme la liste mélange tous les dossiers, chaque ligne affiche son dossier (colonne discrète avant le nombre de messages), et le lecteur l'affiche à l'extrémité droite de la ligne du sujet. Dans « all », le compte suit le dossier (« Archive · perso ») : toujours dans le lecteur, et dans la liste seulement si le sujet garde au moins 30 caractères ; sinon le dossier seul.

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
r répondre (pas encore : contexte édition)
R répondre à tous (pas encore)
f forwarder (pas encore)
d mark done (archiver)
m move (déplacer), puis choisir le dossier. mt déplacer dans le dossier Todo, md déplacer dans le dossier Deleted. Le menu ne propose que les cibles permises depuis le dossier du mail.
del supprimer (déplacer dans Deleted)
t tag (ajouter un tag ; si tous les mails visés l'ont déjà, le retirer)
g group (ajoute l'expéditeur dans un groupe)
l link (panneau des liens du mail ; entrée ouvre le lien dans le navigateur). D'abord les liens qui ont un texte (« Voir ma facture »), puis les numéros de téléphone trouvés dans le mail, avec une icône de téléphone (entrée lance l'appel dans l'app Téléphone du Mac), puis les pièces jointes, enfin les liens sans texte (le plus souvent une image menant à un réseau social). Le panneau est aussi large que la fenêtre le permet, pour lire les liens longs.
u lu / non lu
z annule le dernier déplacement ou la dernière décision (les mails reviennent, la décision précédente aussi) ; chaque z remonte d'une étape
\* drapeau
/ chercher partout : des mots n'importe où, adresses comprises, dans Everything (voir le sélecteur d'attributs) ; échap revient au dossier de départ
? liste des touches
échap annule un préfixe, ferme un panneau, démarque, puis revient du tag, groupe ou contact listé au dossier
espace et ctrl-f font défiler le mail d'une page vers le bas, maj-espace et ctrl-b vers le haut, en texte comme en HTML

Le prefix ";" permet de changer d’écran
; aller à un dossier. Le menu (et la colonne des dossiers) les range en blocs cohérents, séparés par une ligne vide : ce qui est à traiter, ce qui est classé, ce qui a été écrit, ce qui a été jeté ; puis Everything, les écrans Contacts, Tags, Groups, et History à part. Les dossiers propres à l'utilisateur forment un bloc après les dossiers classés. Le nombre de mails non lus de chaque dossier est aligné à l'extrémité droite de sa ligne.

i Inbox
t Todo
w Waiting
q Quarantine

a Archive
n Newsletter
p Paperwork

s Sent
r Drafts

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
	a all attachments : les pièces jointes des mails marqués, ou à défaut du mail sous le curseur ; tous les messages de chaque conversation ; un fichier joint de nouveau à une réponse (même nom, type et taille) n'est enregistré qu'une fois
	g attachments but graphics (gif, png, ...) : de même, sans les images (logos, signatures, photos)
	p as pdf (pas encore)
	e as eml file (dernier mail de chaque conversation, comme t et m)
	t as text file
	m as markdown file
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
