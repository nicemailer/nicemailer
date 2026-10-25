# NiceMailer : liste des domaines de messagerie publics

Statut : proposition à valider. Complète `dossiers.md` (règles de domaine) et `authentification-expediteur.md`. Chiffres relevés le 2026-10-25.

## 1. Objet

Le screening permet d'approuver ou de bannir un domaine entier. Sur un domaine d'entreprise, c'est utile. Sur `gmail.com` ou `orange.fr`, c'est une erreur grave : approuver le domaine approuve des millions d'inconnus.

Le noyau refuse donc toute règle de domaine qui vise un domaine public. Ce document explique comment la liste de ces domaines est constituée et maintenue, et pourquoi certaines sources ont été écartées.

## 2. Définition

Un domaine est **public** quand n'importe qui peut y ouvrir une adresse, sans lien avec le titulaire du domaine :

- webmail gratuit (`gmail.com`, `gmx.fr`) ;
- fournisseur d'accès qui donne des adresses à ses abonnés (`orange.fr`, `free.fr`, `t-online.de`) ;
- messagerie payante ouverte à tous (`fastmail.com`, `hey.com`, `mailbox.org`).

Le critère est le partage du domaine entre des inconnus, pas la gratuité.

## 3. Asymétrie des erreurs

- **Un domaine en trop** (faux positif) empêche seulement la règle de domaine. L'utilisateur peut toujours approuver les adresses une par une. Le coût est faible.
- **Un domaine oublié** (faux négatif) permet d'approuver en bloc tous les utilisateurs d'un fournisseur. Le coût est élevé.

La liste privilégie donc la couverture plutôt que la précision. En revanche, elle n'a pas à contenir des domaines que personne ne demandera jamais à approuver, comme ceux des messageries jetables (voir §4.4).

## 4. Sources évaluées

| Source | Taille | Maintenance | Licence | Retenue |
|---|---|---|---|---|
| HubSpot, *free domains* | 4 768 | Active | Non précisée | Oui, comme base |
| Kikobeats/free-email-domains | 31 034 | Quotidienne | MIT | Oui, seulement pour retirer les domaines morts |
| willwhite/freemail | 4 466 | Abandonnée depuis 2020 | ISC | Non |
| Listes de domaines jetables | 10 000 à 90 000 | Variable | Variable | Non |
| Liste ajoutée à la main (`extra.txt`) | ~110 | Par nous | — | Oui |

### 4.1 HubSpot

C'est la liste des domaines gratuits que les formulaires HubSpot peuvent refuser. Presque toutes les listes libres en dérivent. HubSpot la publie sous forme de CSV à une URL stable.

**Qualités**

- Elle ne vise que les messageries gratuites. Elle ne contient ni domaines jetables récents ni domaines de spam générés en masse.
- Les grands fournisseurs mondiaux y figurent, y compris les géants chinois (`126.com`, `163.com`, `139.com`).
- Elle est maintenue par une entreprise qui en a besoin pour son produit.

**Défauts**

- Elle est ancienne et centrée sur les États-Unis. Elle contient beaucoup de domaines fantaisie de l'époque mail.com (`mail2haiti.com`, `petlover.com`), dont environ 440 ne reçoivent plus de mail.
- Elle n'est pas toujours cohérente : on y trouve quelques domaines jetables (`mailinator.com`) et des sous-domaines (`docs.google.com`).
- Elle couvre mal l'Europe. Il y manque notamment `bbox.fr`, `numericable.fr`, `aliceadsl.fr`, `club-internet.fr`, `neuf.fr`, `gmx.fr`, `hispeed.ch`, `movistar.es` et `mailbox.org`.
- Sa licence n'est pas précisée. Pour une liste de faits, le risque est faible, mais il existe.

### 4.2 Kikobeats/free-email-domains

C'est un paquet npm publié chaque jour. Il fusionne la liste HubSpot, cinq listes de domaines jetables et une centaine de domaines ajoutés à la main (surtout des FAI japonais, nord-américains et norvégiens).

Ses domaines morts sont relevés par DNS : un domaine sans enregistrement MX, ou avec un MX nul, est retiré. Ce relevé est publié à part, dans `inactive-domains.json`.

**Qualités**

- Maintenance quotidienne et automatisée, avec ses propres garde-fous (nombre minimal de domaines par source, contrôle des serveurs DNS).
- Licence MIT.
- Le relevé des domaines morts est fiable et réutilisable indépendamment du reste.

**Défauts**

- Les domaines jetables représentent 85 % du total. On y trouve des milliers de domaines générés (`001310.xyz`, `000webmail.ddnss.de`) qui changent tous les jours. La liste devient illisible et chaque mise à jour produit un gros diff.
- Le paquet ne publie que le résultat fusionné : on ne peut pas savoir de quelle source vient un domaine.
- Le projet ne dépend que d'une personne.

### 4.3 willwhite/freemail

**Qualités** : fichiers séparés pour les domaines gratuits et les domaines jetables.

**Défauts** : aucune mise à jour depuis 2020, et des données corrompues (une ligne `404: not found` dans la liste).

### 4.4 Listes de domaines jetables

`disposable-email-domains`, Castle, FakeFilter, etc. Elles servent à refuser les adresses temporaires lors d'une inscription.

Elles sont écartées pour notre usage. Une règle de domaine est toujours demandée par l'utilisateur pour un expéditeur dont il a reçu un mail. Personne ne demandera d'approuver `mailinator.com` en bloc, et ces listes ajoutent des dizaines de milliers de domaines sans protéger de rien.

### 4.5 Pistes écartées

- **Filtrer selon la forme du nom** (chiffres, extensions `.xyz` ou `.top`). Cela éliminerait `126.com` ou `163.com`, qui font partie des plus grandes messageries au monde.
- **Filtrer par popularité web** (liste Tranco des sites les plus visités). Les domaines de FAI comme `wanadoo.fr` ou `club-internet.fr` ont peu de trafic web et seraient éliminés à tort.
- **Thunderbird ISPDB** (configuration automatique des comptes). Cette base contient de bons domaines de FAI, mais elle mélange fournisseurs grand public et hébergeurs, et son format XML demande un tri manuel. Elle peut servir de réservoir quand on complète `extra.txt` à la main.

## 5. Constitution de la liste

```
  HubSpot (free domains)
− Kikobeats (inactive-domains.json)
+ data/public-domains/extra.txt
− data/public-domains/exclude.txt
= data/public-domains/public-domains.txt
```

Au 2026-10-25, cela donne 4 362 domaines (62 Ko). 33 domaines de `extra.txt` ne figurent dans aucune source.

### Fichiers

| Fichier | Rôle | Modifié par |
|---|---|---|
| `scripts/update-public-domains.py` | Script de génération, Python sans dépendances | Développeur |
| `data/public-domains/extra.txt` | Domaines toujours présents, groupés par région. Ils servent aussi de témoins : les grands noms doivent y rester même si une source les perd | Développeur |
| `data/public-domains/exclude.txt` | Domaines à retirer. Une entrée y l'emporte sur les sources et sur `extra.txt` | Développeur |
| `data/public-domains/public-domains.txt` | Résultat versionné, intégré au build | Le script uniquement |

### Garde-fous du script

Le script n'écrit rien et sort en erreur dans les cas suivants :

- une source ne répond pas, renvoie une page HTML ou contient trop peu de domaines (3 000 pour HubSpot, 100 pour le relevé Kikobeats) ;
- `extra.txt` ou `exclude.txt` contient une entrée qui n'est pas un nom de domaine valide ;
- la nouvelle liste perd plus de 5 % des entrées de l'actuelle. Après vérification, `--force` lève ce blocage.

Le fichier de sortie est trié, sans date. Il n'est réécrit que s'il change, si bien qu'un diff Git ne montre que des changements réels. L'option `--dry-run` affiche les ajouts et retraits sans écrire.

## 6. Utilisation par le noyau

- La liste est embarquée dans `nm-core` par `include_str!`, puis chargée une fois dans un `HashSet`. Les lignes vides et celles qui commencent par `#` sont ignorées.
- La comparaison se fait en minuscules, sur le domaine **et sur chacun de ses parents** : `mail.orange.fr` est refusé parce que `orange.fr` est dans la liste. Sans cela, il suffirait d'un sous-domaine pour contourner la liste.
- La liste ne fait que refuser la création d'une règle de domaine. Elle ne change rien au screening par adresse.
- Elle ne suffit pas à elle seule. Une université ou une très grande entreprise n'est pas un domaine public, mais l'approuver en bloc pose le même problème. Il faut garder l'heuristique de l'interface : ne proposer une règle de domaine qu'après au moins deux adresses approuvées sur ce domaine, et confirmer en affichant le nombre d'expéditeurs concernés.

## 7. Maintenance

- Lancer `scripts/update-public-domains.py` avant chaque release, et relire le diff de `public-domains.txt` avant de le commiter.
- Quand un utilisateur signale un domaine public manquant, l'ajouter à `extra.txt` dans la bonne région.
- Quand un domaine légitime d'entreprise est bloqué à tort, l'ajouter à `exclude.txt`.

## 8. Questions ouvertes

- **Fréquence.** Faut-il automatiser la mise à jour (launchd, ou CI quand le dépôt aura un remote), ou s'en tenir à une exécution avant chaque release ?
- **Licence HubSpot.** Faut-il accepter une source sans licence, ou repartir de Kikobeats (MIT) en filtrant ses domaines jetables, au prix d'un script plus complexe ?
- **Domaines internationalisés.** Le script n'accepte que des noms ASCII (punycode compris). Aucune source actuelle ne contient de domaine accentué, mais le noyau devra convertir le domaine de l'expéditeur en punycode avant la comparaison.
- **Dépendance à Kikobeats.** Si le projet s'arrête, son relevé des domaines morts n'évoluera plus. La liste restera valable, mais grossira lentement de domaines inactifs. Il faudrait alors refaire ce relevé DNS dans notre script.
