# Process Scheduler

autor: Alexandru Sima (322CA)

## Descriere

Programul simulează un planificator de procese care rulează pe un sistem de operare. Acesta este chemat de sistem,
primind motivul pentru care este oprit și întoarce ce trebuie să facă sistemul de operare în continuare.

Planificatorul alocă fiecărui proces o cuantă de timp, după care acesta este preemptat și se trece la următorul. După
execuția unui apel de sistem, procesul poate ceda procesorul (pentru anumite apeluri de sistem, de ex. Sleep sau Wait),
sau poate rămâne, dacă timpul rămas este suficient.

Pentru a măsura trecerea timpului, planificatorul reține un ceas intern, care este înaintat la fiecare interogare,
astfel:

- dacă procesul a fost preemptat (i-a expirat cuanta de timp alocată), ceasul este înaintat cu timpul de la ultima
  actualizare;
- dacă procesul executat un apel de sistem, se calculează timpul trecut ținând cont de timpul alocat rămas și de
  faptul că apelul de sistem durează 1 unitate de timp;

Selecția procesului care rulează în continuare depinde de algoritmul de planificare, care poate fi:

- Round Robin: procesele sunt rulate în ordine.
- Priority Queue: procesele sunt rulate în ordinea priorității, iar dacă două procese au aceeași prioritate, se
  execută în ordinea în care au fost adăugate în coadă. Prioritatea este atribuită la crearea procesului și este
  modificată dacă procesul este preemptat.
- Completely Fair Scheduler (CFS): va fi rulat procesul cu `vruntime`-ul (timpul cât a rulat) cel mai mic. La crearea
  unui proces nou, acesta va primi vruntime-ul minim, pentru a fi rulat urmatorul. În plus, cuanta de timp alocată
  proceselor nu este fixă, ci este împărțită la numărul de procese active, astfel încât fiecare proces să primească o
  parte aproximativ egală din cuantă.

------------------------------------------------------------------------------------------------------------------------

## Implementare

Modulul conține următoarele fișiere:

- `process_manager.rs`: structuri pentru implementarea planificatorului, independent de algoritmul de planificare;
- `round_robin.rs`: structuri pentru implementarea algoritmului Round Robin;
- `priority_queue.rs`: structuri pentru implementarea algoritmului Priority Queue (Round Robin cu priorități);
- `cfs.rs`: structuri pentru implementarea algoritmului CFS;

În `process_manager.rs` sunt definite trait-ul `ProcessInformation`, care definește interfața pe care trebuie să o 
implementeze o structură care stochează un proces pentru a putea fi gestionat de planificator, și structura generică 
`ProcessManager`, al cărei tip depinde de tipul proceselor stocate (acestea trebuind să implementeze 
`ProcessInformation`), care se ocupă de gestionarea întreruperilor primite, de măsurarea timpului și de planificarea 
proceselor (realizată în funcție de tipul proceselor stocate).

În fișierele `round_robin.rs`, `priority_queue.rs` și `cfs.rs` este definită câte o structură care să stocheze procesele
pentru algoritmul respectiv și care implementează metodele care se ocupă de planificarea proceselor, pentru a fi 
folosite de către `ProcessManager` (de exemplu, modul în care se alege următorul proces). Pentru a ușura anumite 
operații, au fost implementate anumite traituri pentru unele structuri de procese, cum ar fi `PartialOrd` pentru a 
determina un criteriu de ordonare (dependent de algoritm) și `Add` și `AddAssign` pentru a adăuga mai facil valori unei 
structuri. În plus, mai sunt definite și structurile wrapper (`RoundRobin`, `PriorityQueue`, `CFS`) care încapsulează 
planificatoarele împreună cu algoritmii lor de planificare, pentru a putea fi folosite în afara modulului.