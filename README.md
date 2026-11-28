# Hypolia Lobby

Hub du réseau : spawn, PNJ, menus, profil, collectibles, comptes FerrisKey et Capitainerie.

Images publiées :

- `ghcr.io/hypolia/lobby` : le plugin, lancé en sidecar à côté du serveur.
- `ghcr.io/hypolia/lobby-server` : rspigot avec le template de `server/`.

Dépend de [`hypolia/sdk`](https://github.com/hypolia/sdk), cloné dans `../sdk` et figé par `sdk.ref`, et de rspigot, attendu dans `/opt/minecraft/dev/rspigot` et figé par `rspigot.ref`.

## Lancer en local

Serveur rspigot (`:25566`, gRPC `:25580`) :

```bash
unzip -o server/world.zip -d server
/opt/minecraft/dev/rspigot/target/release/rspigot --world-dir ./server
```

Plugin :

```bash
cp .env.example .env
cargo run --release -p hypolia-lobby
```

Le proxy Velocity se lance depuis [`hypolia/platform`](https://github.com/hypolia/platform).

## Construire les images

```bash
docker buildx bake
```

Taper `op <pseudo>` dans la console rspigot pour obtenir `/lobby` et `/build`.

## Importer une map Vanilla 26.x

Couper rspigot avant :

```bash
cp -R /chemin/vers/votre_map/* ./server/world/
cargo run --release -p hypolia-lobby --bin normalize-anvil -- ./server/world
```

## Commandes en jeu

Joueurs :
- `/menu` : menu principal
- `/options` : paramètres
- `/sheepwars` : rejoindre SheepWars
- `/spawn` : retour au spawn
- `/login`, `/logout`, `/whoami` : compte FerrisKey
- `/caplogin`, `/caplogout` : compte Capitainerie
- `/mp <joueur> <message>`, `/r <message>` : messages privés

Administration (`op` ou permission `hypolia.admin`) :
- `/build`
- `/lobby setspawn`, `/lobby setpoi <nom>`, `/lobby setpedestal <1..7>`
- `/lobby setleaderboard` : place le tableau flottant du top 10 des victoires (SheepWars ou TowerWars, au clic droit)
- `/lobby setcaptain`, `/lobby setstation <index>`, `/lobby setwaypoint <pnj> <index>`
- `/lobby reload`, `/lobby info`
