IDENTIFICATION DIVISION.
PROGRAM-ID. BAD-SYNTAX.

DATA DIVISION.
    WORKING-STORAGE SECTION.
    01 SOME-STR PIC X(2) VALUE "Overflow!".

PROCEDURE DIVISION.
    DISPLAY "Hello world, I'm " SOME-STR.
STOP RUN.